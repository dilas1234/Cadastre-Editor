-- Phase 4: Aumentum's Mutation Manager setting (MCServer dbo.cadastreGeneralSettings id 1,
-- keepParentIdAfterMutation = true on PRODUCTIONMC). With it on, a Separation may keep the
-- parent's UPIN and attribute record on one of its parts (Aumentum Editor and Map User Guide,
-- "Separation"); every other mutation gives new UPINs. No new table: one setting row.
INSERT INTO cadastre.app_setting (key, value, source)
VALUES ('aumentum.Mutation.keepParentIdAfterMutation', 'true', 'aumentum')
ON CONFLICT (key) DO NOTHING;
