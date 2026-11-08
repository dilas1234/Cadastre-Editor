# 07 — Security, identity, remote access and deployment

## 1. Identity

| Population | Sign-in | MFA | Roles from |
|---|---|---|---|
| Applicants / agents | e-mail or phone + password, or OTP only | SMS/e-mail OTP; optional NIN/BVN match (upgrade) | portal account |
| Staff (officers, Accounts, admins) | **one staff identity** (OIDC SSO), backed by the Registry accounts. Same user names as the Registry, verified against `AuthenticationService` / the Alfresco password hash, as the Cadastre already does | TOTP or passkey for Back Office from outside the LAN; optional on the LAN | Registry / Alfresco groups (Intake Clerk, Cashier, Lands Officer, RofO & CofO Reviewer, Deeds Registrar, Surveyor General, …) + new groups (Accounts, Executive PA) |
| **Executives** (Governor, Commissioner, Perm Sec, SG, Director TP, Registrar) | staff identity | **passkey (FIDO2/WebAuthn) on registered devices: required**; it is also the signing authorisation | groups Governor, Commissioner, Permanent Secretary, … (already defined as Aumentum swimlanes) |
| Services (Bridge, Signing, Certificate) | service accounts + mutual TLS | — | least privilege; `portal-bridge` Registry user in only the groups it needs |

**Joiners and leavers:**
- A staff member leaving is disabled once in the staff identity. That blocks the Back Office, the Desk and signing, and the Registry account is disabled through the existing Users & Units dashboard.
- For signers, the signing certificate is revoked the same day (`04` §3).

## 2. Remote access (executives at home or travelling)

- Executives use the **Executive Desk in the DMZ over HTTPS**, with no VPN.
- Nothing from the Internet reaches the LAN. The Desk exchanges work with the LAN only through the DMZ queues, which the LAN services pull (`01` §2).
- **Controls:**
  - registered devices only (passkey bound to the device);
  - at most 3 devices per person;
  - remote revoke;
  - geo/IP anomaly alerts (sign-in from a new country → second factor + notify ICT);
  - 15-minute idle timeout on mobile;
  - re-authentication for every signature;
  - view-only documents on mobile, watermarked with the viewer's name;
  - no copy or print on mobile; download only on desktop and only for authorised roles.
- **Lost phone:** the executive or their PA calls ICT, who revoke the device in one step. Their key is safe in the HSM.

## 3. Data protection (NDPA 2023) and records

- **Personal data:** names, IDs, addresses, phone numbers, signature images.
  - Encrypted at rest: database and file store; HSM for keys.
  - TLS 1.2+ in transit (except the legacy LRS43 SQL link, which stays on the LAN).
- **Consent and notices** on the portal. A data-protection officer is named. A breach-response procedure exists.
- **Public Verify page:** initials only (`05` §4).
- **Retention:**
  - land records: permanent;
  - portal drafts not submitted: 12 months;
  - logs: 7 years, or as Records Management decides.
- **Audit:** every view, change, signature and payment action is in **append-only, hash-chained logs**, retrievable per file for courts or investigations.

## 4. Application security

| Area | Measures |
|---|---|
| Web | WAF, OWASP ASVS L2 checks, CSP, rate limits, CAPTCHA on public forms, file-upload scanning (ClamAV) and PDF sanitising |
| Payments | gateway webhook signature check + server-to-server verify; no card data stored (PCI scope with the gateway) |
| Signing | WYSIWYS hash check; role / stage check against the live Registry task; per-signature authorisation; HSM-only keys |
| Integration | mTLS between DMZ and LAN; service accounts; idempotency keys; replay protection |
| Legacy findings to fix (`../aumentum_Registry/06`) | stock JBoss console accounts on .5; tickets that never expire; SQL Server 2012 / TLS 1.0; .5 binds JBoss to all interfaces. Restrict to the LAN and change the default credentials |
| Testing | security test (penetration test) of the DMZ apps before go-live and yearly |

## 5. Deployment and resilience

| Item | Plan |
|---|---|
| Hosts | **.11/.127:** application server (exists). **DMZ host:** new VM or VLAN. **Second app node** (VM) for failover. **HSM:** network appliance, or USB HSM ×2 |
| Runtime | systemd services as for `cadastre-server` (or the existing microk8s if ICT prefers); one release pipeline; blue/green restart |
| Databases | PostgreSQL 16 primary on .11 + streaming replica on the second node; nightly dumps off-site. LRS43: restore the regular full + log backups (gap since 2026-10-06) before any automation |
| Files | upload store and certificate store replicated to the second node; nightly off-site copy |
| Monitoring | health of each service, Bridge outbox backlog, signing failures, gateway webhook failures, reconciliation differences, Registry and Cadastre availability (red banner pattern), certificate verification volume |
| Availability targets | Portal / Verify 99.5 %; Executive Desk 99.5 %; Registry-dependent actions follow .5's availability (queued, never lost) |
| Disaster recovery | RPO 15 min (PostgreSQL WAL shipping), RTO 4 h; tested quarterly |
| Environments | **test** (copy of LRS43 + JBoss + Cadastre + HSM test partition + gateway sandbox), **pilot**, **production**. Nothing is first tried on production |
