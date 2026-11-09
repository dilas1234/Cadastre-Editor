# Aumentum Registry (.5) replica — requirements and design

Requirement gathering for replacing Aumentum Registry on 10.10.10.5 (Web Access + Workstation) with a new
application in a separate repository, on the same database LRS43 / `LRSAdmin`. No code is built here.

Start with 01. The files are:

1. [01 Overview and target design](01-overview-and-target-design.md): what .5 is, the target architecture, and the decisions D1–D8
2. [02 Database](02-database.md): every table and the rules for writing to LRS43
3. [03 API catalogue](03-api-catalogue.md): every SOAP operation, authentication, and the replica contract
4. [04 Workflows](04-workflows.md): every process stage by stage, with diagrams and volumes
5. [05 Workstation](05-workstation.md): the Registry Desktop and Cashier, every page and field
6. [06 Web Access and server](06-web-access-and-server.md): the .5 server, admin UI, reports, documents, Web Access
7. [07 Integrations](07-integrations.md): Cadastre, map, Cashier, and the deep links
8. [08 Capture checklist](08-capture-checklist.md): what to copy from .5 before building
9. [09 Build phases](09-build-phases.md): phases R0–R10 and upgrades U1–U8
10. [10 Alfresco model](10-alfresco-model.md): document store, queue, users and permissions inside Alfresco
11. [11 Workstation UI](11-workstation-ui.md): every desktop screen, with reconstructed images in `screens/workstation/`
12. [12 Web UI](12-web-ui.md): Web Access and the admin pages, with reconstructed images in `screens/web/` and real ones in `screens/real/web/`
13. [13 Real capture guide](13-real-capture-guide.md): what to record on a Registry PC, so the real screens can replace the reconstructions
14. [14 Exact workflows](14-workflows-exact.md): every process from Aumentum's own workflow definitions, with roles, every way out, real usage counts and the full route to a C of O

Images in `screens/web/` and `screens/workstation/` are drawn from the code. They are marked "not a real screenshot". Images in `screens/real/` are real screenshots, with field values blurred.
Sources:
- the captures in `tools/aumentum-inventory/lrs-export` and `lrs-capture` (git-ignored);
- the decompiled Registry Desktop client;
- the live WSDLs.

Every fact names its source file. No passwords or personal data are in these documents.
