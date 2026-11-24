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
| V2 | CAD import: binary DXF, DWG through the ODA File Converter, choosing the parcel's layer / polyline | **built** |
| V3 | Adjoining-coordinates report: neighbours sharing a boundary, their corners, as print, CSV and DXF | open |
| V4 | Save the verified boundary (VER number, file number, check report) and hand it to Digitising | open |

## V1: the view and check-only mode

**Sign-in.** Workstation *Checking and Data Verification* on the sign-in page, address
`/verification`. Same access as Digitising and Title Deed: the Users or Administrators group.

**The panel.** The view opens on the *Checking and Data Verification* window:

- **Import CAD / coordinates…** opens the import window. After *Preview*, each closed boundary in
  the file (DXF polylines, LandXML parcels) and the file's points in order (a coordinate list) has
  a **Check** button: it becomes the sketch and is checked at once. There is no *Import into job*
  in this view. Coordinates are taken as UTM 32N metres, as drawn. (V2: DWG and DXF drawings,
  below.)
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

## V2: CAD import (DWG and DXF)

In the unit's import window a `.dwg` or `.dxf` is sent as it is to `POST /api/v1/import/cad?filename=…`
(the file is the body; 16 MiB at most). Nothing is saved.

| File | Read by |
|---|---|
| ASCII DXF | the DXF reader (`cadastre-import/src/dxf.rs`) |
| Binary DXF (R12 and later) | the same reader, through its binary decoder (`dxf::parse_bytes`) |
| DWG (any AutoCAD version) | the **ODA File Converter** on the server, to an AutoCAD 2018 ASCII DXF, then the reader |

**Parcels drawn as loose lines.** Survey drawings often draw a parcel as separate LINEs (or open
polylines) rather than one closed polyline. Each layer's lines are chained into the closed rings
they enclose (PostGIS `ST_Polygonize`, ends snapped to the millimetre; faces under 0.5 m² are
noise), listed as `<layer> ring N (from lines)` beside the drawing's own closed polylines.

**Choosing the parcel.** The window lists the boundaries, closed first and largest first, with
their layer, corners and area, and a **Layer** filter when there are several layers. *Check* loads
one as the boundary and checks it. The whole drawing is shown on the map (magenta, dashed, beacon
labels from its text) for reference while the boundary is chosen or redrawn; *Hide* in the panel,
or *Clear*, removes it.

**Warnings.**
- Coordinates outside Plateau State in UTM 32N (E 150 000–850 000, N 750 000–1 350 000) are named:
  the drawing is on a local grid, Minna / Nigeria grid, or in millimetres, and the check would look
  in the wrong place. Ask the surveyor for UTM 32N (WGS 84); drawings are not reprojected.
- Blocks (`INSERT`) are not exploded; the window says how many there are. Explode them in AutoCAD
  if the parcel is drawn inside a block.

### Installing the ODA File Converter on the server (.11)

DWG needs the converter; DXF does not. Without it, a DWG is refused with a message asking for a
DXF (HTTP 409), and everything else works.

1. Download the Linux package (`.deb`, "Ubuntu 64-bit") from the Open Design Alliance's ODA File
   Converter page (free; accept the licence there).
2. Install it: `sudo apt install ./ODAFileConverter_*.deb`. It installs `ODAFileConverter` in
   `/usr/bin`, where the server finds it with no setting.
3. Try it without a screen, as the server runs it:

   ```bash
   mkdir -p /tmp/oda/in /tmp/oda/out && cp some-plan.dwg /tmp/oda/in/
   QT_QPA_PLATFORM=offscreen ODAFileConverter /tmp/oda/in /tmp/oda/out ACAD2018 DXF 0 1 "*.DWG"
   ls /tmp/oda/out   # some-plan.dxf
   ```

4. If that fails for want of a display, run it under a virtual one: `sudo apt install xvfb`, make
   `/usr/local/bin/oda-headless` (`chmod +x`):

   ```sh
   #!/bin/sh
   exec xvfb-run -a /usr/bin/ODAFileConverter "$@"
   ```

   and start the server with `CADASTRE_ODA_CONVERTER=/usr/local/bin/oda-headless` in its settings.
   `scripts/restart-server.sh` carries the running server's settings forward, so set it once in
   `.env` and start the server from it (stop it first; the script then reads `.env`).

`CADASTRE_ODA_CONVERTER` may also name the program when it is not on the `PATH` (on Windows,
`C:\Program Files\ODA\ODAFileConverter <version>\ODAFileConverter.exe`). One conversion may take
two minutes at most; the converter's own error (`.err` file) is shown when a drawing will not convert.

Tests: `cad_import_reads_dxf_binary_dxf_and_dwg` (integration: lines chained into a ring, binary
DXF read as ASCII, DWG through a stand-in converter), `oda::tests` (command line, `.err` reason) and
the DXF reader's binary tests. The real ODA File Converter is not part of the tests.
