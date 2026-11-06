# 08 — Roadmap to a fully digital lands system

Each phase ends with a pilot on real files and a sign-off by PLAGIS. The phases connect the three existing plans:
- the **Cadastre roll-out** (phases 0–8, this repo, largely live);
- the **Registry replica** (R0–R10, `../aumentum_Registry/09`);
- the **Applicant Portal** (P0–P7, `../registration_Portal/01` §13).

| Stage | Name | Delivers | Depends on | Done when |
|---|---|---|---|---|
| **D0** | Foundations and decisions | <ul><li>Ministry of Justice opinion on e-signature, e-C of O and electronic Land Register (`04` §5)</li><li>gateway contract</li><li>verify domain</li><li>HSM / PKI option chosen</li><li>DMZ host</li><li>**LRS43 backups fixed**</li><li>test environment (LRS43 copy + JBoss + Cadastre)</li></ul> | PLAGIS, State ICT, MoJ | Signed decisions; test environment running |
| **D1** | Identity and Bridge (read) | <ul><li>staff SSO over the Registry accounts</li><li>Bridge reading the Registry: queues per stage, status mirror, fee quotes</li><li>`plagis_ext`</li></ul> | D0 | Executive queues (read-only) match the Registry Desktop queues for every executive role |
| **D2** | Executive Desk (review) | dashboards for Commissioner, Perm Sec, Governor, SG, Director TP, Registrar: <ul><li>documents, map, history, minutes</li><li>**Perm Sec endorsement** (recorded)</li><li>notifications</li></ul> | D1 | Executives use it to review and minute; signing still in ink |
| **D3** | Signing Service and stamps | <ul><li>HSM, CA or provider</li><li>signer register, signature images, stamp library, templates, admin console (four-eyes), timestamps</li><li>PAdES signing, verify</li></ul> | D0 | Test documents validate in Adobe and on the Verify page |
| **D4** | E-signing internal approvals | <ul><li>Commissioner e-signs Land Form 5 and recommendation memos</li><li>SG e-signs TDP and survey confirmations</li><li>Director TP e-signs SAR</li><li>the Bridge attaches the signed PDFs and completes the Registry task</li><li>return-with-minute flow</li></ul> | D2, D3, Bridge write path | 50 files signed remotely with the Registry moving correctly; no paper for these stages |
| **D5** | Applicant Portal and payments | portal P2–P5: online application, gateway, Accounts confirmation, e-receipts, automatic lodgement, tracker, online acceptance of terms | D1, `../registration_Portal` | Pilot form APCO1, then First Registration |
| **D6** | Digital certificates and verification | <ul><li>Certificate Service, QR, Verify page, wallet</li><li>**paper C of O with QR** (if the law still requires ink)</li><li>certificate register updated from Registry events</li></ul> | D3 | Every C of O issued carries a working QR |
| **D7** | Governor e-signing of C of O | Governor view with batch signing; digital C of O as the original (once D0 allows) | D4, D6, legal approval | A full batch signed remotely; owners download and verify |
| **D8** | Paperless back office | <ul><li>mobile site inspection (Town Planning)</li><li>digital charting & reporting</li><li>automatic Scan Outgoing / Delivery</li><li>e-file only (no file jackets for new files)</li><li>certified true copies online</li></ul> | D4–D7 | No printing/scanning for new files |
| **D9** | Backlog and legacy | <ul><li>backfill QR certificates for existing titles on request (Recertification)</li><li>retire .6 (Aumentum Cadastre)</li><li>move from the JBoss Registry to the Registry replica (R10) behind the same Bridge</li></ul> | Registry replica | .6 off; JBoss off; all services unchanged |

**Early wins** (can start now):
- **D2 (review dashboards):** the Bridge read side is mostly patterns that already exist in this repo (`registry_sync.rs`, the registry queues).
- **SG queue:** 1,849 files open at "SG Review Submitted Survey Plan" today.
- **Title deed queue:** already built in the Cadastre (`GET /api/v1/registry/title-deed-queue`) and can feed the SG and Registrar dashboards directly.
