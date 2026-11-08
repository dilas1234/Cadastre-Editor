# 04 — E-signatures, official stamps and keys

## 1. What is signed, by whom, and at which level

| Level | Used for | Technology |
|---|---|---|
| **Simple e-signature** | applicant declaration, acceptance of terms, minutes, endorsements inside the system | identity (OTP / passkey) + audit record; no certificate |
| **Advanced e-signature (AdES)** | every official document: Land Form 5, memos, R of O, C of O, title deed plans, site analysis reports, registration endorsements, certified true copies, receipts, letters | **PAdES-B-LT** (signature + certificate chain + timestamp + revocation data inside the PDF), key in an HSM, one key per signer |
| **Organisation seal ("stamp")** | the official seal of an office, applied automatically or by an authorised officer | a **seal certificate** belonging to the office, not a person (e.g. "PLAGIS Deeds Registry"), plus the seal image |

Every signed document carries both parts:
- a **visible appearance**: the signature image, the name, title and date, the stamp or seal image, and a QR;
- a **cryptographic signature**: tamper-evident, and it validates in Adobe Reader and in the PLAGIS Verify page.

## 2. Signers and their signature appearances

| Signer | Appears on | Visible block |
|---|---|---|
| Governor | C of O, Governor memos | signature image · "Governor of Plateau State" · date · **State seal** |
| Commissioner | Land Form 5, recommendation memos, GR review memo, covering memo | signature image · "Honourable Commissioner, Ministry of Lands, Survey and Town Planning" (title configurable) · date · **Ministry seal** |
| Permanent Secretary | endorsement on the minute sheet / memo | signature image · "Permanent Secretary" · date |
| Surveyor General | confirmation of survey plan, title deed plan | signature image · "Surveyor General" · date · **Survey seal** |
| Director of Town Planning | site analysis report | signature image · title · date |
| Deeds Registrar | registration endorsement on the C of O (instrument no., volume, page), certified true copies | signature image · "Registrar of Deeds" · **Deeds Registry stamp** with the registration particulars |
| Lands officers / reviewers | "prepared by" / "checked by" on Land Form 5 and the C of O draft | name, rank and date (signature optional) |
| System seals 🤖 | receipts, acknowledgements, demand notices, search letters | **PLAGIS system seal** (organisation certificate) + QR |

## 3. Stamps and seals: administration

An **admin console** sits in the Back Office. It is for ICT and Registry admins, and every action needs a **second admin to approve** (four-eyes).

| Function | Detail |
|---|---|
| **Signer register** | person → staff account → role/title → stages they may sign → signature image → signing certificate → validity dates |
| **Signature image enrolment** | the signer draws on a tablet, or uploads a scan of their ink signature. It is cleaned (transparent background) and shown to the signer for approval, and stored encrypted. Changing it needs the signer's re-confirmation + second admin |
| **Stamp / seal library** | State seal, Ministry seal, Survey seal, Deeds Registry stamp, PLAGIS seal, "CERTIFIED TRUE COPY" stamp, "RECEIVED" date stamp, "CANCELLED/REPLACED" stamp. Each has an image (vector/PNG), the office it belongs to, a **seal certificate**, and who may apply it |
| **Templates and placement** | per document type: where the signature and stamp go (page, x/y, size), which signers in which order, and the text lines (title, date format, registration particulars) |
| **Certificates and keys** | issue, renew or revoke signer and seal certificates. Keys are generated **inside the HSM**; admins never see them |
| **Delegations** | acting officers: from, to, stages and dates. The signature shows "for <principal>" |
| **Audit** | every enrolment, change, key operation and signature, with the reason and the approver |
| **Revocation** | revoke a signer (leaves office) or a seal; documents already signed stay valid with a timestamp, and new signing stops at once |

## 4. Keys and certificates (PKI)

| Option | Description | When |
|---|---|---|
| **A. PLAGIS internal CA + HSM** (recommended to start) | a Plateau State / PLAGIS root CA (offline) and an issuing CA. A **network HSM** (e.g. Thales Luna / Utimaco), or a USB HSM in the signing server, holds the signer keys | Signatures validate in the PLAGIS Verify page; Adobe shows "validity unknown" unless the root is trusted (the root can be published, and the Verify page is the authority) |
| **B. Nigerian licensed certificate / trust service provider** | certificates from a provider licensed by NITDA (Nigerian trust-services framework), with remote signing (Cloud Signature Consortium API) | Signatures trusted more widely; per-signature or per-user fees; must be configurable |
| **C. Hybrid** | executives (Governor, Commissioner, Registrar) on a licensed provider; system seals and officers on the internal CA | Recommended target once PLAGIS has budget |

**In all options:**
- **Timestamps** come from an RFC 3161 time-stamping authority: internal at first, and a licensed provider for option B/C.
- **Long-term validation (LTV):** the certificate chain, the revocation status and the timestamp are embedded, so signatures stay verifiable after a certificate expires.
- **Keys are per person.** No shared "Commissioner key" is used by staff: the PA prepares, and only the principal's passkey can authorise their key.

## 5. Legal basis (for PLAGIS / Ministry of Justice to confirm)

- **Recognition in law:** the **Evidence Act 2011 (s. 93)** recognises electronic signatures. The **Cybercrimes (Prohibition, Prevention, etc.) Act 2015 (s. 17)** gives electronic signatures legal effect, with listed exceptions.
- **What must be confirmed:** whether the Governor's consent and execution of statutory instruments under the **Land Use Act 1978** (R of O, C of O, consents) may be signed electronically, and whether a paper original is still required. Other Nigerian states issue electronic or digitally verifiable C of Os, which shows the route is workable.
- **Request:** PLAGIS should obtain a **written opinion from the Plateau State Ministry of Justice**. Possibly an executive order or regulation is needed, covering:
  - e-signature by the Governor and the Commissioner;
  - the electronic Land Register;
  - the legal status of the digital certificate and its printed copy.
- **Until then:** run the system with e-signatures for **internal approvals** (Land Form 5, memos, TDP, SAR) and issue a **paper C of O with the same QR** (signed in ink and scanned), so verification works from day one (`05` §5).
- **Data protection:** signature images and identity data are personal data under the **Nigeria Data Protection Act 2023**. They are stored encrypted, with access logged.

## 6. Signing Service (component E)

| Function | Detail |
|---|---|
| API (LAN; pulled from the DMZ queue) | `sign(document_hash, signer, intent, auth_assertion, appearance_template)` → signed PDF; `seal(document, seal_id)`; `verify(pdf)`; `batch_sign([...])` |
| Checks before signing | signer active; certificate valid; role allowed for this **Registry stage** (read from the Bridge: the task is open and at that stage); document hash = the hash shown to the signer; passkey assertion fresh (< 2 min) |
| Output | PAdES-B-LT PDF, signature id, certificate serial, timestamp token |
| Store | PostgreSQL `signing`: signers, certificates, images (encrypted), stamps, templates, delegations, signature log (immutable, hash-chained) |
| HSM | PKCS#11; per-signer key, non-exportable; in pairs, or with an encrypted backup for disaster recovery |
