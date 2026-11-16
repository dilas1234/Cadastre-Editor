-- The charting queue finds a file's parcels by property number (charting::FILES): without this
-- index every completed job scanned all of parcel_attribute, and the queue took minutes.
CREATE INDEX IF NOT EXISTS parcel_attribute_property_number_idx ON cadastre.parcel_attribute (property_number);
