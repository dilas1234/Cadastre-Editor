-- The parcel Status captured on the "Update Parcel Attribute" dialog (Aumentum: Active, etc.),
-- carried on the draft until commit writes it into cadastre.parcel_attribute.
ALTER TABLE cadastre.parcel_draft_attribute ADD COLUMN IF NOT EXISTS status text;
