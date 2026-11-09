# 13 — Guide for the real Workstation recording and screenshots

The pictures in `screens/workstation/` are reconstructions. Use this guide to replace them with the real screens.

- **Where to record:** a Registry PC (Registry Desktop) with a normal officer account. A second, admin account is needed for the admin items.
- **Which files:** use a **test or trial file** where possible, for example the trial file PLS000138850 / PL67363. Don't record real applicants' details if it can be avoided. If a real file is used, I blur names before anything goes into the repo.
- **How:** a screen recording, like the charting videos (PXL_… .mp4), is best. The officer says each step aloud ("now I am at Review Documents, I click Next"). Screenshots (Win + Shift + S) also work.
- **What to send:** copy the files to this PC and tell me the folder. I'll extract the frames, label them by screen number (column "Replaces"), blur personal data and file them in `screens/real/workstation/`.

## A. Shell and general (5 minutes)

| # | Show | Replaces |
|---|---|---|
| A1 | Start the program: splash, then the login dialog with an empty password | 09, 01 |
| A2 | Main window with the work queue (your tasks), each docked panel, and the status bar | 02 |
| A3 | Open each menu in turn: File, Edit, View, Tools, Help. Hover over each toolbar button | 03–06 |
| A4 | Tools → Options; Help → About; change-password dialog (cancel it) | 37–39 |
| A5 | Right-click on a queue row (context menu); sort and group a queue column | 02 |

## B. One First Registration file (APRO), lodgement to R of O. This is the most important part

| # | Stage / page | Replaces |
|---|---|---|
| B1 | New transaction → Add Transactions page (transaction type list) | 07, 10 |
| B2 | Applicants page, then Add Applicant dialog with its search | 11, 41, 42 |
| B3 | Properties page, then Add/View Property dialog with all its tabs | 12, 43, 44, 50 |
| B4 | Fee page and Set Fee; Payment Info dialog with the **bottom half** shown | 13, 17, 47 |
| B5 | Print Documents page, with the preview of one printed document (acknowledgement / demand notice) | 14 |
| B6 | Complete Task page: open the **transition list** and the **assignee list** and read out every option | 15, 23 |
| B7 | The next stages of the same file (Review Documents, parties, RRR, Register C of O / R of O), one page each | 16, 18–20 |
| B8 | Map button on a property: the map window and the property link | 34, 35 |
| B9 | Scanning: scan or add one page, then indexing | 33 |
| B10 | History of the file | 36 |

## C. Other actions (cancel each one before saving)

| # | Show | Replaces |
|---|---|---|
| C1 | Postpone transaction dialog | 22 |
| C2 | Withdraw: the confirm message, then the rejection-reasons page | 52, 21 |
| C3 | Wizard Cancel message | 53 |
| C4 | Outstanding / ground-rent page with its **penalty section** (any APCO file) | 30, 48 |
| C5 | Search transaction (SEAR) pages | 27, 49 |
| C6 | Survey plan checking (SPC) pages | 28, 29 |
| C7 | Data correction (DCT) pages | 25, 32 |
| C8 | Indexing & scanning (INSC) past-transactions page | 24 |

## D. Admin and fees (admin account)

| # | Show | Replaces |
|---|---|---|
| D1 | Fee structure, fee rate and fee calculation screens | 54–56 |
| D2 | Workflow diagram view | 51 |
| D3 | The **Cashier** web pages: every page, including the receipt | not yet drawn (not captured) |

## E. Files to copy from the same PC (no recording needed)

- the `bin\WorkflowConfig\` folder, including `SubsequentRegistration\` and `Lease.config` / `ReleaseLease.config`
- `WorkstationHelp.htm`
- any `LRS.Data.Controls.dll`

Copy them to a USB stick or a share; see 08 items C1 and C2.

## F. Web Access (I do this myself when the PC is on the PLAGIS LAN)

Still to capture:
- the results grid
- the transaction detail: General, History and every section
- property, party and document details
- the document viewer
- the reports tab

These are read-only, searched by exact numbers only. They also include the admin pages under `/lrs/faces/...` (`screens/real/web/`).
