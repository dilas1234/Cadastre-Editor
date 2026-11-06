-- A beacon number can repeat when the digitizer has seen the other property and given a reason:
-- the first holder keeps duplicate_seq 0, so imports and lookups by number behave as before.
ALTER TABLE cadastre.beacon ADD COLUMN IF NOT EXISTS duplicate_seq integer NOT NULL DEFAULT 0;
ALTER TABLE cadastre.beacon DROP CONSTRAINT IF EXISTS beacon_number_key;
ALTER TABLE cadastre.beacon DROP CONSTRAINT IF EXISTS beacon_number_dup_key;
ALTER TABLE cadastre.beacon ADD CONSTRAINT beacon_number_dup_key UNIQUE (number, duplicate_seq);
