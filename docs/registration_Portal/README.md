# Registration Portal: paperless land registration linked to the Aumentum Registry

- The **application forms** (APRO, APSL, APCO, APCA, DEV, PSUR, individual and organisation) and the State logo are in this folder.
- [01 Portal architecture and workflow](01-portal-architecture-and-workflow.md) covers:
  - what changes and what does not;
  - the architecture: portal (DMZ), payment gateway, back office, and the Registry Bridge (LAN), which lodges into Aumentum through its own services;
  - the end-to-end flow, from application and payment to Accounts confirmation and e-receipt, then lodgement (PLS / PL numbers), every Aumentum stage, online acceptance and the C of O;
  - payments, numbering, data model, validation, documents, notifications, security, decisions for PLAGIS and the build phases P0–P7.
- [03 Automatic lodgement: review and sync](03-lodgement-sync.md) reviews how Aumentum lodges today and gives the Bridge's step-by-step replay. It covers idempotency, locking, client-rule parity, numbering, the Registry → portal mirror, two-way actions, failure handling and how to prove sync before go-live.
- [02 Form → Registry mapping](02-form-to-registry-mapping.md) maps every field, document and fee on the forms to Registry entities and dictionary codes.

These documents build on `../aumentum_Registry/`: workflows (14), database (02), APIs (03), integrations (07).

**Diagrams** (PNG, plus SVG to edit):
- [Architecture](images/portal-architecture.png)
- [Automatic lodgement and sync flow](images/lodgement-sync-flow.png)
