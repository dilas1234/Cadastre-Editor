-- Point the Aumentum Registry (LRS43 on 10.10.10.5) at the new Cadastre on its server, 10.10.10.11 (moved from the PC 10.10.10.9 on 2026-11-04).
-- Run in SQL Server Management Studio on the Registry's SQL Server, database LRS43, as a DBA.
-- Saves the current values first (table LRSAdmin.lr_setting_backup_trial); undo with
-- registry-settings-revert-to-aumentum.sql. Registry desktops must log out and in again
-- (each desktop reads these settings once per session).
USE LRS43;
SET XACT_ABORT ON;
BEGIN TRANSACTION;

-- The backup table already exists from the trial switch; it keeps the original Aumentum values.
IF OBJECT_ID('LRSAdmin.lr_setting_backup_trial') IS NULL
    SELECT Id, category, name, value, GETDATE() AS saved_at
    INTO LRSAdmin.lr_setting_backup_trial
    FROM LRSAdmin.lr_setting
    WHERE (category = 'mcintegration' AND name IN ('GISServer', 'LWAMapURL'))
       OR (category = 'multicadastre' AND name = 'mcServiceAddress');

SELECT 'before' AS stage, category, name, value FROM LRSAdmin.lr_setting
WHERE (category = 'mcintegration' AND name IN ('GISServer', 'LWAMapURL')) OR (category = 'multicadastre' AND name = 'mcServiceAddress');

UPDATE LRSAdmin.lr_setting SET value = 'http://10.10.10.11:8080/GISViewer/MapWorker.aspx'
WHERE category = 'mcintegration' AND name IN ('GISServer', 'LWAMapURL');
UPDATE LRSAdmin.lr_setting SET value = 'http://10.10.10.11:8080/ApplicationServerAdmin/WebServices/LRSWebServices.asmx'
WHERE category = 'multicadastre' AND name = 'mcServiceAddress';

SELECT 'after' AS stage, category, name, value FROM LRSAdmin.lr_setting
WHERE (category = 'mcintegration' AND name IN ('GISServer', 'LWAMapURL')) OR (category = 'multicadastre' AND name = 'mcServiceAddress');

-- Expect 3 rows in "after" with the 10.10.10.11 addresses, then:
COMMIT TRANSACTION;
