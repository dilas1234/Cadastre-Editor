-- Miniature ArcSDE geodatabase mirroring PRODUCTIONMC's sde structure, for testing the read-only
-- export (aumentum-export.ps1) against a local SQL Server. Expected results are in
-- test/expected.txt. State tree: 0 (base) -> 50 -> 100 (DEFAULT); 50 -> 200 (an unposted job version).
IF DB_ID('sde') IS NOT NULL BEGIN ALTER DATABASE sde SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sde; END;
CREATE DATABASE sde;
GO
USE sde;
GO
CREATE SCHEMA sde;
GO
CREATE TABLE sde.SDE_versions (name nvarchar(64), owner nvarchar(32), state_id bigint, status int, parent_name nvarchar(64), creation_time datetime, description nvarchar(64));
CREATE TABLE sde.SDE_states (state_id bigint PRIMARY KEY, lineage_name bigint);
CREATE TABLE sde.SDE_state_lineages (lineage_name bigint, lineage_id bigint);
CREATE TABLE sde.SDE_table_registry (registration_id int, owner nvarchar(32), table_name nvarchar(128), object_flags int, rowid_column nvarchar(32));
INSERT sde.SDE_versions VALUES ('DEFAULT','SDE',100,1,NULL,'2016-01-01',NULL), ('Plateau_State_JobID_X','SDE',200,0,'DEFAULT','2026-10-01',NULL);
INSERT sde.SDE_states VALUES (0,0),(50,50),(100,100),(200,200);
INSERT sde.SDE_state_lineages VALUES (100,0),(100,50),(100,100),(200,0),(200,50),(200,200);
INSERT sde.SDE_table_registry VALUES (11,'SDE','PLATEAU_STATE_PARCELS',1,'OBJECTID'),(153,'SDE','PARCELATTRIBUTETABLE',1,'OBJECTID'),(155,'SDE','PLAGIS_COORDINATES',0,'OBJECTID');

CREATE TABLE sde.PLATEAU_STATE_PARCELS (OBJECTID int, UPIN nvarchar(50), Status smallint, JobID uniqueidentifier, PlanNo nvarchar(50), CadZone nvarchar(50), PlotNo nvarchar(50), BlockNo nvarchar(50), SHAPE geometry);
CREATE TABLE sde.a11 (OBJECTID int, UPIN nvarchar(50), Status smallint, JobID uniqueidentifier, PlanNo nvarchar(50), CadZone nvarchar(50), PlotNo nvarchar(50), BlockNo nvarchar(50), SHAPE geometry, SDE_STATE_ID bigint);
CREATE TABLE sde.D11 (SDE_STATE_ID bigint, SDE_DELETES_ROW_ID int, DELETED_AT bigint);
DECLARE @sq geometry = geometry::STGeomFromText('POLYGON((488200 1100050, 488230 1100050, 488230 1100080, 488200 1100080, 488200 1100050))', 32632);
INSERT sde.PLATEAU_STATE_PARCELS VALUES
 (1,'PL000000000001',2,'11111111-1111-1111-1111-111111111111','PSG 12','Z1','15','B3',@sq),
 (2,'PL000000000002',2,NULL,NULL,NULL,NULL,NULL,@sq),           -- deleted in state 50
 (9,'0',1,NULL,NULL,NULL,NULL,NULL,@sq);                          -- placeholder: never a UPIN
INSERT sde.a11 VALUES
 (3,'PL000000000003',2,NULL,'PSG 13',NULL,'16','B3',@sq,50),     -- added in DEFAULT lineage
 (5,'PL000000000005',2,NULL,NULL,NULL,NULL,NULL,@sq,50),          -- added then deleted in 100 -> history only
 (4,'PL000000000004',1,NULL,NULL,NULL,NULL,NULL,@sq,200);         -- unposted job version: not current, still issued
INSERT sde.D11 VALUES (0,2,50), (50,5,100);

-- Versioned, base empty, NO deletes table (as on production).
CREATE TABLE sde.PARCELATTRIBUTETABLE (OBJECTID int, DatasetName nvarchar(150), RegionName nvarchar(150), ParcelID uniqueidentifier, UPIN nvarchar(50), ParcelName nvarchar(50), Parent nvarchar(50), PropertyNumber nvarchar(50), PropertyType nvarchar(50), Plot nvarchar(50), Block nvarchar(50), Location nvarchar(50), ParcelArea nvarchar(50), AreaUnit smallint, PlanNumber nvarchar(50), LGA nvarchar(50), Status nvarchar(50), CanRollback smallint, IsStrata smallint, MutationType smallint, CreationDate datetime2, LastEditDate datetime2, ConsolidationDate datetime2, ArchiveReason nvarchar(250), ArchiveDate datetime2, LRSID nvarchar(20), LinkageStatus smallint, LinkageDate nvarchar(12), SurveyorGeneral nvarchar(50));
SELECT * INTO sde.a153 FROM sde.PARCELATTRIBUTETABLE WHERE 1 = 0;
ALTER TABLE sde.a153 ADD SDE_STATE_ID bigint;
INSERT sde.a153 (OBJECTID, DatasetName, RegionName, UPIN, PropertyNumber, Plot, Block, PlanNumber, LGA, Status, MutationType, CreationDate, LRSID, LinkageStatus, LinkageDate, ArchiveReason, SDE_STATE_ID) VALUES
 (1,'Plateau_State','Plateau_State','PL000000000001','PRP-1','15','B3','PSG 12','Jos North','Active',0,'2017-03-01T10:00:00','LRS-000981',1,'20170302', NULL, 50),
 (3,'Plateau_State','Plateau_State','PL000000000003','PRP-2','16','B3','PSG 13','Jos North','Active',0,'2018-05-04T09:30:00',NULL,0,NULL, 'note with | pipe and' + CHAR(10) + 'newline', 50),
 (7,'PlateauTest','PlateauTest','PLT000000000001',NULL,NULL,NULL,NULL,NULL,'Active',0,NULL,NULL,0,NULL,NULL,200);

CREATE TABLE sde.PLAGIS_COORDINATES (OBJECTID int, Alias nvarchar(20), X numeric(12,3), Y numeric(12,3));
INSERT sde.PLAGIS_COORDINATES VALUES (1,'ABUZ',488215.125,1100080.5), (2,'E2',500000,1100000);
GO
