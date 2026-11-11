-- Overlaps narrower than this (metres) are a shared boundary drawn a little differently (the same
-- beacons typed from the survey plan against a neighbour's stored line), not land taken twice:
-- the system check, the Orthophoto badge and the commit's no-overlap rule let them through.
-- 0 counts every overlap over 0.01 m², as before.
INSERT INTO cadastre.app_setting (key, value, source) VALUES
    ('editor.overlap_min_width_m', '0.05', 'cadastre')
ON CONFLICT (key) DO NOTHING;
