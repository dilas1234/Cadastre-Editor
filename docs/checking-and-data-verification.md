# Checking and Data Verification unit

The unit checks a surveyor's work **before** any Digitising job exists, on the same platform as
the digitisers. Source: the unit's handwritten procedure (17 Nov 2026), summarised below.

| | The unit's procedure |
|---|---|
| 1 | Copy the surveyor's soft copy (AutoCAD format) to the desktop. |
| 2 | Import / add the CAD file into the work environment. |
| 3 | Digitise the AutoCAD work with our shapefiles and save it. |
| 4 | Load all committed work and check for adjoining and overlapping parcels; make the necessary correction. |
| 5 | Where there is adjoining, query the adjoining coordinates from the database and give them to the surveyor for conformity. |
| "Second platform" | The same checks, except that nothing is saved after checking. |

Decisions (23 Nov 2026): the unit is a separate stage before the Digitising unit's job; the
check-only mode is a mode of this editor; both DWG and DXF are imported (DWG through the ODA File
Converter on the server); adjoining coordinates go to the surveyor as a printed list, CSV and DXF;
a verified boundary is tied to its file by the Registry file number when known and always by its
own VER number; anyone who may open Digitising may open this unit.

## Build phases

| Phase | What | State |
|---|---|---|
| V1 | The unit's view (`/verification`) and check-only mode | **built** |
| V2 | CAD import: binary DXF, DWG through the ODA File Converter, choosing the parcel's layer / polyline | open |
| V3 | Adjoining-coordinates report: neighbours sharing a boundary, their corners, as print, CSV and DXF | open |
| V4 | Save the verified boundary (VER number, file number, check report) and hand it to Digitising | open |

## V1: the view and check-only mode

**Sign-in.** Workstation *Checking and Data Verification* on the sign-in page, address
`/verification`. Same access as Digitising and Title Deed: the Users or Administrators group.

**The panel.** The view opens on the *Checking and Data Verification* window:

- **Import CAD / coordinates…** opens the import window. After *Preview*, each closed boundary in
  the file (DXF polylines, LandXML parcels) and the file's points in order (a coordinate list) has
  a **Check** button: it becomes the sketch and is checked at once. There is no *Import into job*
  in this view. Coordinates are taken as UTM 32N metres, as drawn.
- **Draw boundary** starts a sketch: click the corners, or type / paste them in *Edit Sketch
  Properties*.
- **Check boundary** (also the sketch's *Check Boundary* button, where Digitising has *Finish
  Sketch*) runs the check. Nothing is saved; the boundary stays to be adjusted and checked again.
  Once it is moved, the result says the boundary has changed since the check.

**The check** (`POST /api/v1/verification/check`, `{ "vertices": [[e, n], …] }`, editor role)
is the Digitising system check without a job (`precheck::run` with no job):

| Check | Result |
|---|---|
| Overlap with this Cadastre (committed parcels; strips under `editor.overlap_min_width_m` are a shared boundary) | fail |
| Overlap with Aumentum's live parcels | fail; warn if Aumentum cannot be asked |
| Drop point: is the plot already digitised, here or in Aumentum? | fail |
| Title on the overlapped land | fail |
| Gaps under 10 cm to committed neighbours | warn |

The Registry transaction check, and leaving out the parcels a job replaces or corrects, belong to
a job and are not run. Fewer than 3 corners is refused (400).

Tests: `verification_checks_a_boundary_without_a_job_and_saves_nothing` (integration) and the
view/address cases in `auth.rs` and `tests/login.rs`.
