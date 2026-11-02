-- Miniature LRS43 (schema LRSAdmin) mirroring the Registry's linkage tables, for testing the
-- read-only Registry export (lrs-export.ps1) against a local SQL Server, and the copy loader
-- (tools/import/load-lrs-copy.sh). Expected results are in test/lrs-expected.txt.
IF DB_ID('LRS43') IS NOT NULL BEGIN ALTER DATABASE LRS43 SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE LRS43; END;
CREATE DATABASE LRS43;
GO
USE LRS43;
GO
CREATE SCHEMA LRSAdmin;
GO
CREATE TABLE LRSAdmin.lr_dictionary (Id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), code nvarchar(255), label nvarchar(255), descr nvarchar(1024), category nvarchar(255), general_term tinyint, sort_order int, hidden tinyint, version numeric(19,0), begin_lifespan_version datetime, end_lifespan_version datetime, label_lang nvarchar(255), descr_lang nvarchar(1024), parentcode nvarchar(255));
INSERT LRSAdmin.lr_dictionary (Id, code, label, category) VALUES
 (24, 'transaction_lodged', 'Lodged', 'transactionstatus'), (25, 'transaction_in_progress', 'In Progress', 'transactionstatus'),
 (27, 'transaction_completed', 'Completed', 'transactionstatus'), (28, 'transaction_withdrawn', 'Withdrawn', 'transactionstatus'),
 (520, 'su_active', 'Active', 'spatialunitstatus'), (521, 'su_historic', 'Historic', 'spatialunitstatus');
CREATE TABLE LRSAdmin.lr_setting (Id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), category nvarchar(255), name nvarchar(255), value nvarchar(2000), description nvarchar(1024));
INSERT LRSAdmin.lr_setting VALUES
 (42, 'com.landsystems.lrs.model.Setting', 'mcintegration', 'IntegrationWithMC', '1', 'Allow an Integration with Aumentum Cadastre'),
 (47, 'com.landsystems.lrs.model.Setting', 'mcintegration', 'GISUpdatedFields', 'suID:LRSID,1:LinkageStatus,spatialDate1:LinkageDate,refnum:PropertyNumber,plotNumber:Plot,blockNumber:Block', 'kept: config tables are exported whole');
CREATE TABLE LRSAdmin.lr_address (id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), street_name nvarchar(255), district numeric(19,0), region numeric(19,0));
INSERT LRSAdmin.lr_address VALUES (1, 'Address', 'Rayfield Road', 1, 1), (2, 'Address', 'PERSON HOME ADDRESS - must not be exported', 1, 1);
CREATE TABLE LRSAdmin.lr_spatial_unit (id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), suid nvarchar(255), refnum nvarchar(255), upin nvarchar(255), block_number nvarchar(255), lot_number nvarchar(255), status numeric(19,0), area float, spatial_date1 datetime, note nvarchar(2000), comments nvarchar(2000), address numeric(19,0) REFERENCES LRSAdmin.lr_address (id), version numeric(19,0), begin_lifespan_version datetime, end_lifespan_version datetime);
INSERT LRSAdmin.lr_spatial_unit VALUES
 (1001, 'Parcel', 'PLT00000857', 'PL1049', 'PL000000003002', N'B1 Ọ̀gbọ́mọ̀ṣọ́ é', '12', 520, 864.4665, '2016-05-26T10:11:12.345', 'NOTE - must not be exported', 'COMMENT - must not be exported', 1, 1, '2016-05-26', NULL),
 (1002, 'Parcel', 'PLT00000136', 'PL2000', 'PL000000001075', NULL, 'a|b' + CHAR(13) + CHAR(10) + 'c', 521, 0.1, NULL, NULL, NULL, NULL, 2, '2016-05-26', '2017-01-01');
CREATE TABLE LRSAdmin.lr_transaction (id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), transaction_number nvarchar(255), transaction_type numeric(19,0), transaction_status numeric(19,0), source_property_ids nvarchar(2000), target_property_ids nvarchar(2000), create_date datetime, complete_date datetime, inscription nvarchar(max), inscription2 nvarchar(max), comments nvarchar(2000), reg_initials nvarchar(50), gis_params nvarchar(2000), version numeric(19,0));
INSERT LRSAdmin.lr_transaction VALUES
 (10000000012483, 'Transaction', 'PLS000006909', 1, 27, 'PLT00000857', NULL, '2017-04-11', '2017-05-10', 'TO JOHN DOE - must not be exported', NULL, 'must not be exported', 'JD', NULL, 1),
 (10000000012484, 'Transaction', 'PLS000006910', 1, 24, NULL, NULL, '2026-10-30', NULL, NULL, NULL, NULL, NULL, NULL, 1);
CREATE TABLE LRSAdmin.lr_launit (id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), unit_uid nvarchar(255), note nvarchar(2000));
INSERT LRSAdmin.lr_launit VALUES (2001, 'LAUnit', 'LAU1', NULL);
CREATE TABLE LRSAdmin.lr_launit_spatial_unit (launit numeric(19,0) NOT NULL, spatial_unit numeric(19,0) NOT NULL, unit_index int NOT NULL, CONSTRAINT pk_lsu PRIMARY KEY (launit, unit_index));
INSERT LRSAdmin.lr_launit_spatial_unit VALUES (2001, 1001, 0);
CREATE TABLE LRSAdmin.lr_transaction_launit (transaction_id numeric(19,0) NOT NULL, launit_id numeric(19,0) NOT NULL, launit_index int NOT NULL);
INSERT LRSAdmin.lr_transaction_launit VALUES (10000000012483, 2001, 0);
CREATE TABLE LRSAdmin.lr_mc_job (Id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), job_instance_id numeric(19,0), job_id numeric(19,0), job_priority numeric(19,0), job_name nvarchar(255), job_status int, job_started datetime, job_updated datetime, job_ended datetime, job_comments nvarchar(2000), job_parameters nvarchar(2000), transaction_id numeric(19,0));
INSERT LRSAdmin.lr_mc_job VALUES (1, 'MCJob', 17001, 3, 1, 'Survey Plan', 3, '2017-04-11', '2017-05-10', '2017-05-10', NULL, NULL, 10000000012483);
CREATE TABLE LRSAdmin.lr_payment (id numeric(19,0) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), transaction_number nvarchar(255), is_paid tinyint, receipt_no nvarchar(255), internal_receipt_no nvarchar(255), payment_date datetime, payment_status numeric(19,0), bank_name nvarchar(255), account_name nvarchar(255), account_number nvarchar(255), paid_sum float, comments nvarchar(2000));
INSERT LRSAdmin.lr_payment VALUES (1, 'Payment', 'PLS000006909', 1, '003139071', 'RCT000000001', '2017-04-12', 1, 'BANK - must not be exported', 'ACCOUNT NAME - must not be exported', '0123456789', 25000.5, NULL);
-- An identity column (the loader keeps its values) and a view (structure only).
CREATE TABLE LRSAdmin.lr_tempcounter (Id numeric(19,0) IDENTITY(1,1) NOT NULL PRIMARY KEY, Hjtype nvarchar(255), counter numeric(19,0), value_text nvarchar(255), ref_number nvarchar(255), transaction_id numeric(19,0), transaction_index int);
SET IDENTITY_INSERT LRSAdmin.lr_tempcounter ON;
INSERT LRSAdmin.lr_tempcounter (Id, Hjtype, counter, value_text, ref_number, transaction_id, transaction_index) VALUES (5, 'TempCounter', 1, 'PLS000006910', NULL, 10000000012484, 0);
SET IDENTITY_INSERT LRSAdmin.lr_tempcounter OFF;
CREATE INDEX ix_su_upin ON LRSAdmin.lr_spatial_unit (upin) INCLUDE (suid);
-- Personal data: structure exported, rows never.
CREATE TABLE LRSAdmin.lr_person (id numeric(19,0) NOT NULL PRIMARY KEY, name nvarchar(255), name2 nvarchar(255));
INSERT LRSAdmin.lr_person VALUES (1, 'JOHN', 'DOE');
GO
CREATE VIEW LRSAdmin.tqb_transaction_type AS SELECT Id, code, label FROM LRSAdmin.lr_dictionary WHERE category = 'transactiontype';
GO
