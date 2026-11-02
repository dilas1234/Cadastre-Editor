-- Phase 5: Registry link sync. A cache of the Registry's current properties that hold a UPIN
-- (LRSAdmin.lr_spatial_unit, read through the Registry API), refreshed every reconcile. A link
-- removed in the Registry shows as a property whose UPIN changed between two snapshots; a
-- property linked to several parcels holds only one UPIN, so "differs from the Cadastre" alone
-- is not an unlink. Nothing here is written to the Registry.
CREATE TABLE IF NOT EXISTS cadastre.registry_property (
    su_id       text PRIMARY KEY,           -- Registry property id (SuID, PLT...)
    registry_id bigint,                     -- lr_spatial_unit.id
    upin        text NOT NULL,              -- the Cadastre UPIN the Registry holds
    refnum      text,                       -- Registry document number (PropertyNumber)
    seen_at     timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS registry_property_upin_idx ON cadastre.registry_property (upin);

-- The document type a plan sent from the Cadastre is filed under in the Registry: MCServer's
-- dictionary entry SurveyPlanFromMC (label sd_titledeedPlan, Registry "Title Deed Plan").
INSERT INTO cadastre.app_setting (key, value, source)
VALUES ('aumentum.Integration.SurveyPlanFromMC', 'sd_titledeedPlan', 'aumentum')
ON CONFLICT (key) DO NOTHING;
