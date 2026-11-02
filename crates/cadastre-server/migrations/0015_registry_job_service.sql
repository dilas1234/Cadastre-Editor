-- Phase 5: the Registry's job service (Aumentum ApplicationServerAdmin/WebServices/LRSWebServices.asmx)
-- served by this Cadastre. The Registry approves a job when its transaction is completed and
-- rejects it when the transaction is withdrawn (decompiled lrs-mc-integration IntegrationServiceImpl:
-- approveJob / rejectJob). Aumentum's history agrees: every job with status 5 belongs to a Completed
-- transaction, every job with status 7 to a Withdrawn one (LRS43 lr_mc_job, 2026-11-02). Inferred
-- labels; to be confirmed against Aumentum's ApplicationServerAdmin code once captured.
UPDATE cadastre.wf_job_status SET label = 'Approved by the Registry' WHERE code = 5 AND label = 'Ended (code 5)';
UPDATE cadastre.wf_job_status SET label = 'Rejected by the Registry' WHERE code = 7 AND label = 'Ended (code 7)';
