# 02 — End-to-end digital workflow (application → verified certificate)

The stages and roles are Aumentum's own, from `../aumentum_Registry/14-workflows-exact.md`. The table shows First Registration (APRO/APSL/CONT/RAOL/REGR), the longest route; the other processes use the same building blocks (§3).

**Legend:**
- ✍ e-signature
- 🔏 official stamp (seal)
- 🤖 automatic (Bridge / service)
- 📱 can be done from phone, tablet or desktop (Executive Desk)

## 1. First Registration, digital

| # | Stage (Aumentum task) | Who | System | Paper today → digital | Sign / stamp | Server |
|---|---|---|---|---|---|---|
| 0 | Apply, upload documents, pre-checks, fee quote | Applicant | Portal | paper form + photocopies → online wizard + uploads | applicant declaration by OTP ✍ (simple e-signature) | DMZ, reads .5 via Bridge |
| 0b | Pay, then Accounts confirm | Applicant, Accounts | Portal + gateway + Back Office | bank teller slip → gateway + e-receipt | e-receipt sealed by the system 🔏🤖 | DMZ / .11 |
| 1 | Lodge Application (`LodgmentApplication`) | Bridge 🤖 | Registry | counter keying → automatic | lodgement letter sealed 🔏🤖 | .5 |
| 2 | Scan Incoming Documents | Bridge 🤖 | Registry | scanning → none (born digital) | — | .5 |
| 3 | Open New Property File (PL no.) | Lands Officer | Registry Desktop | physical file jacket → e-file | — | .5 |
| 4–5 | Awaiting Fee Payment → Register Processing Fee Payment and Generate Receipt | Bridge 🤖 (after Accounts) | Registry | cashier keying → automatic | receipt 🔏🤖 | .5 |
| 6 | SG Review Submitted Survey Plan | Surveyor General 📱 | Executive Desk (or Desktop) | file moved to the SG desk → queue | — (review) | .5, .11 |
| 7 | Compare and Verify Survey Plan (charting) | Map Compilation Officer | Cadastre + charting form | ArcMap + paper report → digital charting report (`../charting-and-reporting.docx`) | officer ✍ on the charting report | .11 (.6 legacy) |
| 8 | Confirm Survey Plan | Surveyor General 📱 | Executive Desk | ink → e-confirm | SG ✍ | .5 |
| 9–14 | Site analysis: review request → schedule → inspect → report → review → approve | Director TP 📱, TP Coordinator, Inspection Officer | Back Office + **mobile inspection form** (photos, GPS) | paper report → digital Site Analysis Report (`sd_SAR`) | Inspection Officer ✍, Director TP ✍ 📱 | .11 → .5 |
| 15 | Review Submission | Lands Officer | Registry Desktop | — | — | .5 |
| 16 | Prepare Land Form 5 (recommendation) | Lands Officer | Registry Desktop → PDF | typed and printed → generated PDF | Lands Officer ✍ (prepared by) | .5 |
| 17 | Review Land Form 5 | R of O & C of O Reviewer | Registry Desktop | — | Reviewer ✍ (checked by) | .5 |
| 17b | **Permanent Secretary endorsement** (see §2) | Permanent Secretary 📱 | Executive Desk | file walked to the Perm Sec → dashboard | Perm Sec ✍ | DMZ ↔ .11 ↔ .5 |
| 18 | **Approve Land Form 5** (`ApproveLandForm5`) | **Commissioner 📱** | Executive Desk | ink signature on a file that travels → e-sign anywhere | Commissioner ✍ + Ministry seal 🔏 | DMZ ↔ .11 ↔ .5 |
| 19 | Review Signed Land Form 5 | Reviewer | Registry Desktop (signed PDF attached) | — | — | .5 |
| 20 | Prepare R of O Acceptance of Terms & Conditions of Grant | Lands Officer | Registry → published to Portal | printed letter → portal document | Lands ✍ / Director of Lands ✍ (as per law) | .5 → DMZ |
| 21 | Awaiting Acceptance of Statutory R of O in Principle | **Applicant** | Portal (accept with OTP) | signed paper form → online acceptance | applicant ✍ (OTP) | DMZ → .5 |
| 22 | Register CofO Fees Payment and Generate Receipt | Bridge 🤖 after Accounts | Registry | as 4–5 | receipt 🔏🤖 | .5 |
| 23 | Raise Title Deed Plan: Receive, Review and Assign | Surveyor General 📱 | Executive Desk | — | — | .5 |
| 24 | Prepare Title Deed Plan in the Cadastre (digitise, **UPIN**) | Digitising (VDES) | **Cadastre .11** | — (already digital) | — | .11 (jobs from .5) |
| 25 | Review Title Deed Plan (SQCO) | SQCO officer | Cadastre / Back Office | paper checking report → digital | SQCO ✍ | .11 |
| 26–27 | Review and Approve Title Deed Plan (SG) | Surveyor General 📱 | Executive Desk | ink on the plan → e-sign | SG ✍ + Survey seal 🔏 on the TDP (`sd_Signed_TDP`) | .11 → .5 |
| 28 | Scan Title Deed Plan | Bridge 🤖 | Registry | scan → signed TDP attached directly | — | .5 |
| 29 | Generate Certificate of Occupancy | Deeds Registrar | Registry → Certificate Service draft | printed on security paper → draft PDF with TDP, terms, parties | — | .5 → .11 |
| 30 | Review Certificate of Occupancy | Reviewer | Registry Desktop / Back Office | — | Reviewer ✍ (checked by) | .5 |
| 30b | (optional) Commissioner recommendation to the Governor | Commissioner 📱 | Executive Desk | covering memo | Commissioner ✍ | — |
| 31 | **Approve Certificate of Occupancy** (`ApproveCofO`) | **Governor 📱** | Executive Desk (Governor view) | batches of certificates carried to Government House → e-sign anywhere, batch signing | **Governor ✍ + State seal 🔏** | DMZ ↔ .11 ↔ .5 |
| 32 | Review Signed Certificate of Occupancy | Reviewer | Registry Desktop | — | — | .5 |
| 33 | Register Right of Occupancy / C of O (instrument, volume, page) | Deeds Registrar 📱 | Registry Desktop / Executive Desk | register book → digital register | **Registrar ✍ + Deeds Registry stamp 🔏** (registration endorsement) | .5 |
| 34 | Delivery of Documents | Certificate Service 🤖 | Certificate Service → Portal wallet | collection at the office → digital certificate + QR (paper original by appointment if required) | final certificate sealed 🔏 + timestamp | .11 → DMZ |
| 35 | Scan Outgoing Documents | Bridge 🤖 | Registry | scanning → signed PDFs attached directly | — | .5 |
| — | Verify | anyone | Verify page / QR scan | phone call / visit → scan the QR | — | DMZ |

## 2. Permanent Secretary in the workflow

- Aumentum's First Registration process **declares a "Permanent Secretary" role** (swimlane), but **no stage is assigned to it** in any version. Today the Perm Sec's review happens on paper, outside the Registry.
- The application forms describe it: "commissioner … Perm. Secretary … Director of Lands", and later "the Perm. Sec and Commissioner have signed".
- **Design: an endorsement step in the Executive Desk, recorded in `plagis_ext`.** When a file reaches an Approve stage of the Commissioner, it is offered **first** to the Permanent Secretary:
  - the Perm Sec can **Endorse** (✍, with optional minute) or **Return with minute**;
  - the Commissioner sees the endorsement and minute on the same screen;
  - PLAGIS can set the rule per stage: **Perm Sec endorsement required / optional / off**.
- This needs **no change to the Aumentum workflow**. Adding a formal "Perm Sec review" stage to the jBPM process is possible later, once the Registry replica is in place.

## 3. Approval stages across all processes (Executive Desk queues)

From the workflow definitions; the averages and open counts are from `lr_process_history` (2026-11-06).

| Executive | Aumentum stage | Process(es) | Document signed | Today: average time / open now |
|---|---|---|---|---|
| **Commissioner** | Approve Land Form 5 | First Registration | Land Form 5 (`sd_landform_5`) | 1.4 d / 13 |
| **Commissioner** | Approve Recommendation (Commissioner) | Subsequent Registration (26 dealing types, e.g. consent to assign, part surrender, devolution) | Recommendation memo (`sd_recMemo_Com`) | 21.7 h / 7 |
| **Commissioner** | Review and Sign Ground Rent Review Memorandum | Ground Rent Review | GR review memo (`sd_review_groundrent_memo`) | — |
| **Permanent Secretary** | endorsement before each Commissioner approval (§2) | as above | minute + endorsement | — (paper today) |
| **Governor** | Approve Certificate of Occupancy | First Registration, APCO, Recertification, Replacement of Title, Subsequent Registration | C of O (`sd_Co_title`) | 14.7 h / 2 (First Reg.); 3.5 d (APCO) |
| **Governor** | Approve Recommendation (Governor) | Subsequent Registration (e.g. revocation) | Recommendation memo for the Governor (`sd_recMemo_Gov`) | 23.7 d |
| Surveyor General | SG Review Submitted Survey Plan · Confirm Survey Plan · Receive/Review/Approve Title Deed Plan | Survey Plan Checking and Approval, Raise Title Deed Plan | survey confirmation, title deed plan (`sd_Signed_TDP`) | 39.6 d / **1,849 open** at SG Review |
| Director of Town Planning | Review Request for Site Analysis · Review Report and Approve | Raise Site Analysis Report | site analysis report (`sd_SAR`) | 5.1 d / 50 |
| Deeds Registrar | Register C of O / Register Instrument / Certify Documents | all registering processes | registration endorsement, certified true copies | 18.5 h |

The Surveyor General's queue (1,849 files open at "SG Review Submitted Survey Plan", average 39.6 days) shows the biggest gain from a mobile signing desk.

## 4. How an approval on the Executive Desk moves the Registry

1. The Registry reaches an approval stage, for example `ApproveLandForm5`. The Bridge sees the open task in `lr_process_history`.
2. The Bridge collects the documents, the history and the key data, and publishes a **signing package** to the Executive Desk queue. The package holds:
   - the document to sign, as a PDF rendered by the Registry or the Certificate Service;
   - supporting documents and the minute history;
   - the parcel map from the Cadastre;
   - the Perm Sec endorsement, if any.
3. The executive opens it on any device and reads it.
   - **Approve & Sign:** the signing request (document hash + signer + passkey proof) goes through the queue to the **Signing Service**. The Signing Service applies the PAdES signature, the stamp image and a timestamp, and returns the signed PDF.
   - **Return with minute:** no signature. The minute goes to the preparing officer; see §5.
4. On a signature, the Bridge:
   - locks the Registry task;
   - attaches the signed PDF as the stage's outgoing document;
   - completes the task with the configured transition (e.g. "Review Signed Land Form 5"), with the comment "E-signed by <name> via Executive Desk, signature id …";
   - unlocks the task.
5. The Registry carries on as normal. The applicant tracker shows "Approved".

## 5. Returns, and why Aumentum needs care

The Aumentum approval stages have **only a forward transition (and Withdraw)**. There is no "return" path for the Commissioner or the Governor in the workflow definitions.

**Design:**
- **Return with minute** keeps the Registry task **open with the executive**, without completing it.
- The minute goes to the preparing officer's Back Office queue and to the reviewer. The officer corrects the document (regenerate Land Form 5 or the C of O) and **re-submits**. The Bridge replaces the signing package, and the executive sees "Resubmitted after your minute".
- If the application must be refused, the officer **withdraws** the file in the Registry, as Aumentum allows. The Desk only records the minute.
- **Later:** a proper "Return" transition is added when the Registry replica owns the workflow (`../aumentum_Registry/09` R3).

## 6. Other processes

| Process | Digital route (same blocks) |
|---|---|
| **APCO** (C of O for an existing R of O) | Portal → payment → Bridge lodge → Review Submission → Raise Title Deed Plan (SG ✍ TDP) → Generate C of O → Review → **Governor ✍🔏** → Register (**Registrar ✍🔏**) → digital C of O + QR |
| **Subsequent Registration** (consent to assign APCA, part surrender PSUR, devolution DEVO, mortgage, change of name, …) | Portal → payment → lodge → Review Submission → Prepare Recommendation Memo → Review → **(Perm Sec endorse) → Commissioner ✍** (→ **Governor ✍** where required) → Review Signed → Register Instrument (**Registrar ✍🔏**) → digital consent / instrument + QR |
| **Search / CTC** (SEAR, CETC) | Portal → pay → Generate Search Letter (officer ✍) → **certified true copies with the Registrar ✍🔏 + QR** → wallet |
| **Ground Rent Payment** (GRPM) | Portal (pay by property number) → automatic lodgement + receipt 🔏 → done |
| **Ground Rent Review** | memo → **Commissioner ✍** |
| **Recertification, Replacement of Title** | as APCO; Replacement revokes the old certificate's QR (status "replaced") |
| **Indexing & Scanning** (backlog) | internal: Records Clerk / QA in the Back Office. Linking property to parcel uses the new Cadastre |
