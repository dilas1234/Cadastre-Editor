-- Capture property-document attributes for newly digitized parcels, and the job's Registry
-- linkage target, so that at commit each new UPIN is written into cadastre.parcel_attribute with
-- its Registry link (Level A: local linkage, keyed off the job's LRS transaction).

-- Attributes captured while a parcel is still a draft (it has no UPIN yet). These are copied into
-- cadastre.parcel_attribute at commit, keyed by the then-allocated UPIN, and then removed.
CREATE TABLE IF NOT EXISTS cadastre.parcel_draft_attribute (
    parcel_id        uuid PRIMARY KEY REFERENCES cadastre.parcel (id) ON DELETE CASCADE,
    property_number  text,
    property_type    text,
    plot             text,
    block            text,
    location         text,
    plan_number      text,
    lga              text,
    parcel_name      text,
    area_unit        smallint,
    surveyor_general text,
    lrs_id           text,            -- Registry property to link to (defaults from the job)
    updated_at       timestamptz NOT NULL DEFAULT now()
);

-- A job's default Registry linkage target: the property its LRS transaction serves. Applied to
-- every parcel committed under the job unless the parcel captured its own values.
CREATE TABLE IF NOT EXISTS cadastre.wf_job_linkage (
    wf_job_instance_id integer PRIMARY KEY REFERENCES cadastre.wf_job_instance (id) ON DELETE CASCADE,
    lrs_id             text,
    property_number    text,
    property_type      text,
    plot               text,
    block              text,
    plan_number        text,
    lga                text,
    linkage_status     smallint NOT NULL DEFAULT 1,  -- 1 = linked, 0 = pending
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- OBJECTIDs for the parcel_attribute rows we create at commit, kept well above any OBJECTID an
-- Aumentum import could carry so the two never collide.
CREATE SEQUENCE IF NOT EXISTS cadastre.parcel_attribute_objectid_seq START 1000000000;
