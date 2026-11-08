# Form → Registry mapping

This document maps every field and document on the 10 PLAGIS application forms in this folder to the Aumentum Registry (LRS43). The Bridge builds the lodgement from these entities and sends it through the Registry's own services (`ReceptionService.AcceptApplication`, see `../aumentum_Registry/03` §4 and `05` §2.3). It **never writes `LRSAdmin` tables directly**, so the column names below say where each value ends up, not what the Bridge writes.

Codes are taken from the exported `lr_dictionary` (3,704 rows). "**Confirm**" marks a field whose Registry column is not yet proven. Confirm it on the trial pattern: lodge one test file in the Desktop with every field filled, then read it back.

## 1. Forms in this folder

| File | Code | Transaction (`lr_transaction_metadata`) | Aumentum process | Party type |
|---|---|---|---|---|
| APRO1 - Individual Application for Right of Occupancy | APRO1 | `APRO` Application for Statutory Right of Occupancy | First Registration | Individual |
| APRO2 - Organisation Application for Right of Occupancy | APRO2 | `APRO` | First Registration | Organisation |
| APSL1 - Individual Application for State Land | APSL1 | `APSL` Application for State Land | First Registration | Individual |
| APSL2 - Organisation Application for State Land | APSL2 | `APSL` | First Registration | Organisation |
| APCO1 - Individual Application for Certificate of Occupancy | APCO1 | `APCO` Application for Certificate of Occupancy | Application for Certificate of Occupancy | Individual |
| APCA1 - Individual Application for Consent to Assign | APCA1 | `APCA` | Subsequent Registration | Individual |
| APCA2 - Organisation Application for Consent to Assign | APCA2 | `APCA` | Subsequent Registration | Organisation |
| DEV01 - Individual Application for Devolution | DEV01 | `DEVO` Devolution | Subsequent Registration | Individual |
| PSUR1 - Individual Application for Part Surrender | PSUR1 | `PSUR` Part Surrender | Subsequent Registration | Individual |
| PSUR2 - Organisation Application for Part Surrender | PSUR2 | `PSUR` | Subsequent Registration | Organisation |

**Not in the folder:**
- forms referred to by these forms: APCO2 (organisation C of O), DEV02, PSF1 and PSF2 (additional owners);
- forms for the other transaction types: ASSI, MORT, CHPC, EXTT, APRF, SEAR, CETC, REPT, MERT, SUBD and others (`../aumentum_Registry/14` summary).

## 2. Common sections → Registry entities

### 2.1 Applicant, individual (APRO1/APSL1/APCO1 items 2–16; APCA1/PSUR1 4–11; DEV01 3–10)

| Form field | Registry entity . property | Table . column | Dictionary / format |
|---|---|---|---|
| Date of Application | Batch / Transaction application date | `lr_transaction.application_date` | date |
| Title | Individual.salutation | `lr_individual.salutation` | `salutation` (97 values: salut1 Mr., salut3 Mrs., salut4 Dr., salut7 Chief, salut9 Alhaji …) |
| Surname | Individual.lastName | `lr_individual.last_name` | |
| First Name | Individual.firstName | `lr_individual.first_name` | |
| Other Names | Individual.secondName | `lr_individual.second_name` | |
| Date of Birth | Person.birthDate | `lr_person.birth_date` | date |
| Gender | Individual.gender | `lr_individual.gender` | `gender`: male, female |
| Occupation | Individual.occupation | `lr_individual.occupation` | |
| Place of Birth, Nationality, State of Origin, LGA, Home Town | **Confirm**: free / extended fields (`pers1/pers2`, `indiv1/indiv2`, or `lr_party_ext.extended_params`, as configured in `lr_formfield`) | | the portal keeps them in full in its own record |
| Identification Document | Person.personIdType | `lr_person.person_id_type` | `id_document_type`: passport, nat_id, driver_license, v_card, others |
| Identification Number | Person.personIdNumber | `lr_person.person_id_number` | |
| TIN | Person.tin | `lr_person.tin` | |
| Applicant role | Applicant (party) | `lr_party` (Hjtype Applicant) + `lr_applicant` | `applicant_role_type`: applicant / agent; `party_type`: natural |
| Delivery preference | Applicant.deliveryType | `lr_applicant.delivery_type` | `delivery_type`: in person / mail out |

### 2.2 Applicant, organisation (APRO2/APSL2 items 2–18; APCA2/PSUR2 3–19)

| Form field | Registry | Notes |
|---|---|---|
| Name of Organisation | Corporation.name → `lr_corporation.name` | `party_type`: juridical |
| Registration (RC) Number | `lr_corporation.cac_number` | |
| Date / Country of Registration | **Confirm** (`corp1/corp2` or extended params) | |
| TIN | `lr_person.tin` | |
| Website | **Confirm** | portal keeps it |
| Contact person: Designation, Title, Surname, Other names | `lr_corporation.represent_name` (+ representative party) | |

### 2.3 Contact (APRO1 items 17–24 and equivalents)

| Form field | Registry |
|---|---|
| No. / Address | `lr_address.street_number`, `street_name` / `address_line1` |
| City/Town/Village | `lr_address.city` |
| State | `lr_address.region` |
| Country | `lr_address.country` (`country` dictionary) |
| Telephone | `lr_person.phone1_number` |
| Email | `lr_person.email` |
| Preferred mode of communication | portal only (`portal.applicant_account`); used by notifications |

### 2.4 Representative (APRO1 25–32; APRO2 23–30; APCA2/PSUR2 24–31)

| Form field | Registry |
|---|---|
| Title, Surname, Other names, Contact address, Telephone, Email | second Applicant party with `applicant_role_type_agent` (`party_type` representative) |
| Registration particulars of Power of Attorney (Yes/No) | document `sd_POA` / `sd_IPOA` required when Yes |
| Relationship with the applicant | `lr_party.comments` (**confirm**) |

### 2.5 Third-party notification

Kept in the portal only (`portal.application_party` with role `third_party`). These people are notified at completion. Nothing is sent to the Registry.

### 2.6 Property: new land (APRO, APSL; items 33–41)

| Form field | Registry | Dictionary / notes |
|---|---|---|
| Block No. | `lr_spatial_unit.block_number` | |
| Plot No. | `lr_spatial_unit.lot_number` | |
| Area + unit | `lr_spatial_unit.area`, `area_unit_type` | `area_measurement_unit`: hectar, acre, sqm, sqyrd |
| Land Location / Description | `lr_launit.legal_description` / `lr_address` of the property; `lr_spatial_unit.zone` = location | `location`: jos, outsidejos, lgc, rural (this sets the fees) |
| Region / LGA / property area | `lr_address.region`, `local_government_area`; `lr_spatial_unit.property_area` | `region`, `region_lga`, `property_area` (hierarchy by code prefix) |
| Value of Improvement Offered | `lr_launit.build_value` | money |
| Time required for Erection | `lr_launit.build_term` | |
| Land use / purpose (tick list) | `lr_spatial_unit.purpose` / `specific`; `lr_launit.purpose_clause` | `actual_use_type` (14: ause_resid, ause_commer, ause_indust, ause_agr, ause_pubins, ause_recreat …) and `use_specific` (307: ause_resid_1 Low Density Residential, ause_commer_13 Petrol Filling Station …). The form's ~150 tick boxes must be mapped once to these codes (**to do with Lands**) |
| Possesses other R of O / C of O? + certificate number | `lr_party.cert_no` (**confirm**) | used for the duplicate check |
| Property type | `lr_spatial_unit.spatial_unit_type` = Land Parcel | |

The property file number (`PL…`, `lr_launit.unit_uid` = `lr_spatial_unit.refnum`) is issued by the Registry at **Open New Property File**. It is never typed by the applicant for First Registration.

### 2.7 Property: existing (APCO, APCA, PSUR, DEVO)

| Form field | Registry | Portal behaviour |
|---|---|---|
| Property No. | `lr_launit.unit_uid` / `lr_spatial_unit.refnum` (e.g. PL67363) | Looked up live through the Bridge. The portal shows the title owner(s), address, area and land use read-only, and refuses unknown or archived properties |
| Title Owner(s) | current RRR parties (`lr_rrr` + `lr_party`, `title_holder`) | Pre-filled from the Registry. The applicant confirms; they cannot type their own |
| Property Address | property `lr_address` | Pre-filled |
| Original size of land, Land use, Purpose (PSUR) | `lr_spatial_unit.area`, `purpose`, `specific` | Pre-filled; the applicant states the part being surrendered |
| Property Original Reference Co-ordinate (PSUR) | **Confirm**; the composite plan carries the geometry, and the Cadastre handles the split (mutation type 6 "Separated" or 1 "Split") | Upload of the composite plan is mandatory |

### 2.8 Transaction-specific fields

| Form | Field | Registry |
|---|---|---|
| APCA1/2 | Is the applicant also the assignor? | the applicant party is also given role `party_role_type_assignor` |
| APCA1/2 | Assignee(s): name | target parties, role `party_role_type_assignee` (`target_party_type` of the metadata) |
| APCA1/2 | Reason for assignment | `lr_transaction_ext.extended_comments` (**confirm**) |
| APCA1/2 | Consideration amount (figures / words) | **Confirm**: transaction params / extended params. The Desktop's fee page uses it in the consent fee calculation (`transaction_params`) |
| PSUR1/2 | Reason, Valuable consideration | as APCA |
| DEV01 | Is the applicant also the executor/administrator? | applicant role |
| DEV01 | Name on title (deceased) | source party, role `party_role_type_deceased` |
| DEV01 | Executor(s)/Administrator(s) | target parties, role `party_role_type_beneficiary` (**confirm** with Legal) |
| All | Attestation / declaration + signature/date | Tick + OTP in the portal (time, IP, account). The signed declaration PDF is stored as `sd_application_form` |

## 3. Supporting documents → `document_type` codes

| Document on the form | Forms | Registry code |
|---|---|---|
| Taxpayer Identification Number Printout | APRO1, APSL1, DEV01 | **confirm** (no TIN code; use `sd_identification` or ask Lands to add one) |
| Form of Identification / Identification Document | APRO1, APCO1, DEV01 | `sd_identification` (Identification Document) |
| Survey Report | APRO1 | `sd_surbReport_RightOcc` (Survey Report on Right of Occupancy) |
| Passport Photograph | APRO1, APCO1 | `sd_passportPic` |
| Income Tax Clearance | APRO1, APSL2 | `sd_incomeTaxClearance` |
| Change of Ownership | APRO1 | `sd_localGovChOwn` (Local Govt Change of Ownership) |
| Deed of Assignment | APRO1 | `sd_ASG` |
| Customary Evidence of Ownership | APRO, APCO, APSL2, DEV01 | `sd_customary` (Customary Document) / `sd_CRO` (Customary R of O Certificate) |
| Court Judgement | APRO, APCO, APSL, DEV01 | `sd_court_judgement` |
| Power of Attorney / Authorization Letter | all | `sd_POA` / `sd_IPOA` / `sd_aulet` (Authority Letter) |
| Sales Agreement | APRO, APCO, APSL | `sd_sales_agreement` |
| Certificate of Incorporation | APRO2 | `sd_incorpcert` |
| Organisation Representative Authorization Letter | APRO2, PSUR2 | `sd_aulet` |
| Allocation Fee Payment Receipt | APSL1/2 | `sd_evidence_payment`; replaced by the portal receipt when the allocation fee is paid online |
| Tax Clearance Certificate (Assignee / Assignor) | APCA, PSUR | `sd_incomeTaxClearance` (two uploads, labelled) |
| Ground Rent Payment Receipt / Evidence of Last Ground Rent Payment | APCA, PSUR, DEV01 | `sd_GRreceipt` / `sd_evidGRpayment`. Not needed when the Registry shows no outstanding ground rent |
| Consent Fee / Part Surrender Fee / Site Inspection Fee Payment Receipt | APCA, PSUR | **not uploaded**: paid through the portal invoice; the e-receipt is attached by the Bridge as `sd_payment_receipt` |
| Original Certificate of Occupancy | PSUR | `sd_originCert_occu` (scan) and `sd_originCofO` (photocopy) |
| Composite Plan | PSUR | `sd_surveyPlan` (**confirm**: a "composite plan" code may be wanted) |
| Court Affidavit | DEV01 | `sd_courtAff` |
| Cover Letter to the Honourable Commissioner | DEV01 | `sd_honourCommission` |
| Letter of Administration | DEV01 | `sd_LOA` |
| Grant of Probate | DEV01 | `sd_grantProbate` |
| Publication | DEV01 | `sd_publication` |
| Bank Deposit Slip / Teller Printout | DEV01 | `sd_recappfee` / `sd_teller`. Not needed when paid online |
| Memorandum and Articles of Association | DEV01 | `sd_memoassoc` |
| Deed of Will and Testament | DEV01 | `sd_DWI` |
| Death certificate (implied for devolution) | DEV01 | `sd_deathcert` (**ask Legal** whether it is mandatory) |

**Documents the Registry generates**, which the portal publishes to the applicant:
- `sd_lodgement_letter` (Confirmation of Lodgement)
- `sd_demandnotice`
- `sd_payment_receipt`
- `sd_rightOcc_accept` (R of O Acceptance Form; accepted online)
- `sd_landfoem_7` (R of O Title)
- `sd_Co_title` (C of O)
- `sd_reject_letter`
- `sd_delivery_letter`

## 4. Fee types per form (from `cashier_transaction_code`)

| Form | Fee types (`fee_type` codes) |
|---|---|
| APRO1/2 | APFF Application Form Fee, PCOFO Premium of CofO, IPL Infrastructure Provision Levy, COPF, IFCO Issuance Fee for C of O, RCOF, TDPF TDP Fee, SVF Survey Fee, BPAF Building Plan Approval, PLF Plan Lodgement (private survey only), LAF Layout Approval. The form also lists Contravention Fee (CF) and Ground Rent + 5 % penalty |
| APSL1/2 | ALF Allocation Fee, APFF, PCOFO, IPL, COPF, IFCO, RCOF, TDPF, GR Ground Rent |
| APCO1 | APFF, PCOFO, IPL, COPF, IFCO, RCOF, TDPF, SVF |
| APCA1/2 | APFF, PFROFO Processing Fee, GR Outstanding Ground Rent, GRP Ground Rent Penalty (+ CSF Consent Fee per the form) |
| DEV01 | APF Application Fee, TDPF, PFD, PCOFO, IFCO |
| PSUR1/2 | APF, PFPS, TDPF, PCOFO, IFCO, SVF, PLF, PSF Part Surrender Fee (+ SIF Site Inspection Fee per the form) |

Amounts always come from the Registry's fee rules (`lr_fee_structure` + `lr_fee_rate` + Cashier), quoted by the Bridge. The portal never holds its own fee table. Where a form lists a fee that the metadata does not (CF, CSF, SIF), Lands/Accounts must confirm whether it is calculated in the Registry or added by hand.

## 5. To confirm with PLAGIS (Lands, Legal, Accounts)

1. **Land use:** map the ~150 land-use / purpose tick boxes on the R of O forms to `actual_use_type` / `use_specific` codes.
2. **Fields without a column:** where the Registry keeps place of birth, nationality, state of origin, LGA, home town, organisation registration date and country, website, and consideration amount. Check the `lr_formfield` rows used by the lodgement pages (`../aumentum_Registry/05` App. A); fields with no Registry home stay in the portal record and on the attached application PDF.
3. **Fees not in the metadata:** whether CF, CSF and SIF are calculated in the Registry.
4. **Devolution:** the party roles for devolution, and whether a death certificate is mandatory.
5. **Missing forms:** APCO2, DEV02, PSF1/PSF2, and the forms for the remaining transaction types.
