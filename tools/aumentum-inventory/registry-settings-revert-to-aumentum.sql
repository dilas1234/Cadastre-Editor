-- Put the Aumentum Registry back on Aumentum Cadastre (10.10.10.6): restores the exact values
-- saved by registry-settings-switch-to-new-cadastre.sql. Database LRS43, as a DBA.
-- Registry desktops must log out and in again afterwards.
USE LRS43;
SET XACT_ABORT ON;
BEGIN TRANSACTION;

UPDATE s SET s.value = b.value
FROM LRSAdmin.lr_setting s JOIN LRSAdmin.lr_setting_backup_trial b ON b.Id = s.Id;

SELECT 'restored' AS stage, category, name, value FROM LRSAdmin.lr_setting
WHERE (category = 'mcintegration' AND name IN ('GISServer', 'LWAMapURL')) OR (category = 'multicadastre' AND name = 'mcServiceAddress');

-- Expect the 10.10.10.6 addresses again, then:
COMMIT TRANSACTION;
-- When the trial is over for good you may drop the backup: DROP TABLE LRSAdmin.lr_setting_backup_trial;
