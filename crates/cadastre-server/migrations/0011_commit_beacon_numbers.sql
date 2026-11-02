-- Beacon numbers at commit (Aumentum: the officer types each corner's beacon number, from the
-- surveyor's point list, when the parcel is committed; the system never generates one).
-- A setting row only; no new table. 1 = every new corner needs a number to commit.
INSERT INTO cadastre.app_setting (key, value, source) VALUES
    ('commit.require_beacon_numbers', '1', 'cadastre'),
    ('commit.beacon_tolerance_m', '0.05', 'cadastre')   -- a corner this close to a beacon IS that beacon
ON CONFLICT (key) DO NOTHING;
