-- Miniature MCServer (Aumentum application DB) with the real column definitions of the workflow,
-- user, UPIN and region tables, for testing aumentum-export-mcserver.ps1 locally.
IF DB_ID('MCServer') IS NOT NULL BEGIN ALTER DATABASE MCServer SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE MCServer; END;
CREATE DATABASE MCServer;
GO
USE MCServer;
GO
CREATE TABLE dbo.wfjob (id int, job nvarchar(20), description nvarchar(400), startTaskID int, endTaskID int, workflowDesignXML nvarchar(max));
CREATE TABLE dbo.wftask (id int, name nvarchar(50), description nvarchar(400));
CREATE TABLE dbo.wftaskflow (jobID int, taskID int, taskOrder int, nextTaskID int);
CREATE TABLE dbo.Step (id int, name nvarchar(50), iconImage nvarchar(50), commands text, executeOrApplyCommand nvarchar(50));
CREATE TABLE dbo.TaskSteps (taskId int, stepId int);
CREATE TABLE dbo.wftaskPermission (taskID int, sid int, isDefault bit);
CREATE TABLE dbo.wfjobInstance (id int, jobID int, taskID int, sid int, priority char(1), started datetime, updated datetime, ended datetime, name nvarchar(250), COMMENT nvarchar(800), parameters nvarchar(2000), lrsTransactionNumber nvarchar(50), jobstatus int, created datetime, lrsKey nvarchar(50), lrsProperties xml, linkParcelsProperties xml);
CREATE TABLE dbo.wfTaskInstance (id int, jobID int, taskID int, sid int, assigned datetime, started datetime, ended datetime, jobInstanceID int, comment nvarchar(800), parameters nvarchar(2000));
CREATE TABLE dbo.sec_sids (sid int, code nvarchar(64), name nvarchar(128), descr nvarchar(4000), is_user bit, is_locked bit, can_change_pass bit, can_be_expired bit, password nvarchar(128), pswd_date datetime, os_account nvarchar(128), os_sid varbinary(85), is_system bit);
CREATE TABLE dbo.UPIN (id uniqueidentifier, upin nchar(14), beg_date datetime, end_date datetime, status char(1), parcelID uniqueidentifier, regionID uniqueidentifier);
CREATE TABLE dbo.region (id uniqueidentifier, r_name nvarchar(50), r_user nvarchar(50), password nvarchar(500), isDefault bit);
CREATE TABLE dbo.sync_upin_tracking (x int);
INSERT dbo.wfjob VALUES (5,'Parcel Creation','Parcel Creation',13,9,'<?xml version="1.0"?>' + CHAR(13) + CHAR(10) + '<WorkFlow Name="WorkFlow">' + CHAR(10) + '  <Activity Name="Plot Parcel" /></WorkFlow>');
INSERT dbo.wftask VALUES (13,'Plot Parcel','Plot Parcel'),(14,'Review Plotted Parcel','Review Plotted Parcel'),(9,'Commit Parcel','Commit Parcel');
INSERT dbo.wftaskflow VALUES (5,13,1,14),(5,14,2,9),(5,14,2,13);
INSERT dbo.Step VALUES (1005,'Create Job Area','_06_Create_job_area_active','<command>{52D2FA3A-60EF-43bd-A7F9-61C247312080}</command><command>{5F9BC173-CF00-4a3b-8766-E14FF7D462A8}</command>',NULL);
INSERT dbo.TaskSteps VALUES (13,1005);
INSERT dbo.wftaskPermission VALUES (13,2,1),(13,101,0);
INSERT dbo.wfjobInstance VALUES (17001,5,14,101,'N','2026-10-01T09:00:00','2026-10-02T10:00:00',NULL,'Job PLS000021241','urgent | client waiting',NULL,'PLS000021241',1,'2026-10-01T08:59:00','4711','<Properties><P id="981"/></Properties>',NULL);
INSERT dbo.wfTaskInstance VALUES (34001,5,13,101,'2026-10-01T09:00:00','2026-10-01T09:05:00','2026-10-02T10:00:00',17001,'plotted',NULL);
INSERT dbo.sec_sids VALUES (2,'Admins','Administrators',NULL,0,0,0,0,NULL,NULL,NULL,NULL,1),(101,'officer1','Cadastral Officer',NULL,1,0,1,0,'SECRET-HASH','2020-01-01',N'PLAGIS\officer1',0x010500000000000515000000,0);
INSERT dbo.UPIN VALUES (NEWID(),'PL000000023895','2025-12-01',NULL,'A',NEWID(),NEWID()),(NEWID(),'PL000000023901','2026-01-01',NULL,'R',NULL,NEWID());
INSERT dbo.region VALUES (NEWID(),'Plateau_State','sde','ENCRYPTED-SECRET',1);
GO
