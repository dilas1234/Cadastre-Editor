# 05 — Digital certificates and scan-to-verify

## 1. Documents issued digitally

| Document | Registry code | Signed by | When it is issued | Where the owner gets it |
|---|---|---|---|---|
| Payment receipt | `sd_payment_receipt` | PLAGIS system seal | Accounts confirm | Portal wallet, SMS link, e-mail |
| Acknowledgement / lodgement letter | `sd_lodgement_letter` | system seal | lodgement | wallet |
| Demand notice | `sd_demandnotice` | system seal | invoice | wallet |
| R of O acceptance terms (accepted online) | `sd_rightOcc_accept` | applicant (OTP) + Lands | stage 20–21 | wallet |
| **Statutory Right of Occupancy** (Land Form 7) | `sd_landfoem_7` | Commissioner / as per law + seal | after Land Form 5 approval | wallet |
| **Certificate of Occupancy** (with the title deed plan) | `sd_Co_title` (+ `sd_Signed_TDP`) | **Governor ✍ + State seal**; Surveyor General on the TDP; **Registrar's registration endorsement + stamp** | after Register C of O | wallet + optional printed original |
| Consent (assignment, mortgage, …) / registered instrument | outgoing consent / instrument codes | Commissioner / Governor + Registrar | Subsequent Registration | wallet |
| Search letter, certified true copies | `sd_search_report`, `sd_certified_document` | Registrar + "Certified True Copy" stamp | Search / CETC | wallet |
| Rejection / withdrawal letter | `sd_reject_letter` | officer + seal | on rejection | wallet |

## 2. Certificate generation (Certificate Service, component F)

1. At "Generate Certificate of Occupancy", the Deeds Registrar generates the C of O in the Registry as today. The **Certificate Service** produces the **digital C of O**:
   - **PDF/A-3**, laid out as the current official C of O;
   - holder(s), address, term, rent, purpose clause, conditions;
   - the parcel description and **title deed plan** from the Cadastre (UPIN, coordinates, area);
   - the certificate number;
   - a placeholder for the registration particulars.
2. Reviewer checks → Governor e-signs (`03`, `04`).
3. At "Register C of O", the Registrar registers it (instrument / volume / page from the Registry counters). The Signing Service then adds the **registration endorsement + Deeds Registry stamp** as a second signature, and the **QR**.
4. The final file is attached to the Registry as `sd_Co_title` by the Bridge, and published to the owner's wallet. The Certificate Service records it in its **certificate register**.

**Contents of the certificate register** (PostgreSQL `certificate`):
- certificate id;
- type, number, version;
- transaction (PLS), property (PL), UPIN;
- holder names, kept as a **hash** plus initials for the public view;
- issue date, signers, signature ids;
- SHA-256 of the PDF;
- status: valid / replaced / revoked / surrendered (part) / expired (term ended) / under caveat (informational);
- links to the replacing and replaced certificates.

## 3. The QR code

- **Content:** a short **verify URL**, `https://verify.plagis.gov.ng/c/<certificate-id>`.
  - The domain is to be decided with State ICT.
  - The id is a random 128-bit value encoded in base32, so certificates cannot be enumerated.
- **Offline check:** the QR also carries a **compact signed payload**, readable by the PLAGIS app or any QR reader. The payload is an Ed25519 signature by the PLAGIS seal over: certificate number, holder initials, UPIN, area, issue date and the PDF hash. It can be checked with no network access.
- **Placement:** the QR is printed on every page of the certificate and on the title deed plan. A short **verification code** (e.g. `PLC-7KQ4-9M2X`) is printed beside it for people who type it in.

## 4. Verify page (public, DMZ)

Scanning the QR or typing the code shows:

| Shown | Not shown |
|---|---|
| ✅ **Genuine** / ⚠ **Replaced by …** / ⛔ **Revoked** / ❓ **Not found** | the full owner name (initials only, e.g. "J. D."), unless the visitor proves they know it (types the full name to match) |
| Certificate type and number, issue date, signers (Governor / Registrar), registration particulars | ID numbers, addresses, phone numbers |
| Parcel: UPIN, LGA, area, land use; a small map with the parcel outline (optional, PLAGIS to decide) | other parcels' owners |
| **Upload check**: drop the PDF → hash and signatures are verified → "This file is exactly the one PLAGIS issued" or "This file has been altered" | |
| Current encumbrances notice: "There are registered interests on this title. Request a search for details" (Search transaction, paid) | the details of the encumbrances |

**Rules:**
- Every verification is logged, but the visitor is not identified.
- Rate limiting stops scraping.
- **Banks, courts and lawyers** can register for an **API key** to verify in bulk (an upgrade).

## 5. Printed copies (transition and preference)

| Case | What is printed |
|---|---|
| While the law requires an ink-signed original | the official C of O on security paper, signed in ink, **with the same QR and code**. The scanned signed original is attached to the Registry and the certificate register, so scanning the paper confirms it |
| After e-signature is approved | owners may print the digital C of O. The QR and the digital signature make the printout verifiable. An optional **security-paper print** at the office (fee) for owners who want one |
| Lost / damaged | Replacement of Title (REPT) issues a new certificate. The old QR shows **Replaced**, with a link to the new one |

## 6. Life-cycle events reflected on the verify page

| Registry event | Certificate status |
|---|---|
| Registered | Valid |
| Assignment / devolution completed (new holder) | old: **Replaced** (by the new certificate or endorsement) |
| Part surrender | old: **Replaced**; new certificates for the remaining part and the surrendered part |
| Revocation of R of O / C of O (`RROC`, `RCOO`) | **Revoked** |
| Replacement of title | old: **Replaced** |
| Set R of O as expired | **Expired** |
| Caveat lodged / mortgage registered | still Valid, plus the "registered interests" notice |

The Bridge watches these completed transactions in the Registry (`lr_transaction` status 25 + type) and updates the certificate register within a minute.

## 7. Owner wallet (in the Applicant Portal)

- All documents for each property the owner holds or has applied for.
- Download as PDF; share a **time-limited share link** with a bank or lawyer.
- Notifications of any later transaction lodged against the property, as fraud protection: "A Consent to Assign has been lodged on PL67363. If this was not you, contact PLAGIS."
