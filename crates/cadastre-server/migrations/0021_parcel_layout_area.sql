-- The parcel form's Layout and Area (as the Registry property carries Zone, Layout and
-- PropertyArea): kept on the draft's capture and on the committed parcel's attribute record.
ALTER TABLE cadastre.parcel_draft_attribute ADD COLUMN IF NOT EXISTS layout text, ADD COLUMN IF NOT EXISTS area_name text;
ALTER TABLE cadastre.parcel_attribute ADD COLUMN IF NOT EXISTS layout text, ADD COLUMN IF NOT EXISTS area_name text;
