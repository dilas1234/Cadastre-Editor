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
| V2b | File formats: import KML, TIFF, BMP and EMF as well; export DXF, DWG, KML, EMF, TIFF and BMP | **built** |
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
2. Install it with a virtual display: `sudo apt install ./ODAFileConverter_*.deb xvfb xauth`. It
   installs `ODAFileConverter` in `/usr/bin`, where the server finds it with no setting. The
   converter has only Qt's `xcb` plugin, so it needs an X display even though it shows nothing
   useful (`QT_QPA_PLATFORM=offscreen` does **not** work); the server runs it under `xvfb-run`.
3. Try it as the server runs it:

   ```bash
   mkdir -p /tmp/oda/in /tmp/oda/out && cp some-plan.dwg /tmp/oda/in/PLAN.DWG
   xvfb-run -a ODAFileConverter /tmp/oda/in /tmp/oda/out ACAD2018 DXF 0 1 "*.DWG"
   ls /tmp/oda/out   # PLAN.dxf
   ```

No server setting or restart is needed for either package. Without `xvfb` the server's own
`DISPLAY` is used if it has one (a window may flash on that desktop); with neither, a DWG is
refused with a message naming the package (HTTP 409).

`CADASTRE_ODA_CONVERTER` may also name the program when it is not on the `PATH` (on Windows,
`C:\Program Files\ODA\ODAFileConverter <version>\ODAFileConverter.exe`). One conversion may take
two minutes at most; the converter's own error (`.err` file) is shown when a drawing will not convert.

Tests: `cad_import_reads_dxf_binary_dxf_and_dwg` (integration: lines chained into a ring, binary
DXF read as ASCII, DWG through a stand-in converter), `oda::tests` and the DXF reader's binary
tests. `the_real_converter_reads_a_dwg` uses the real converter where it and a display are
installed, and is skipped elsewhere. Checked by hand with ODA 27.10 (24 Nov 2026): DWGs of AutoCAD
2000, 2004, 2010, 2013 and 2018 made by the converter from a simple drawing; not yet with surveyors'
own drawings.

## V2b: file formats, in and out

Nothing is saved on the server by an import or an export.

### Import

| File | How it is read | Where it lies |
|---|---|---|
| DXF, DWG | as V2 | UTM 32N, as drawn |
| KML | `cadastre-import/src/kml.rs`; folders are layers. KMZ is refused (save as KML) | longitude / latitude, projected to UTM 32N |
| EMF | `cadastre-import/src/emf.rs`: lines, curves and text; the pen's colour is the layer. Arcs and images inside it are left out, with a warning; EMF+ only files are refused | page units: placed by points (below). An EMF exported here carries its placement |
| TIFF | the server makes a PNG of it (`src/picture.rs`, `POST /api/v1/import/picture`, 64 MiB): 1-bit (Group 4), grey, palette, RGB, CMYK; first page. Group 3 is refused | GeoTIFF tags, a `.tfw`, or placed by points |
| BMP | the browser itself | a `.bpw`, or placed by points |

- A picture's world file and `.prj` are chosen **together with it** in the file box. Without a
  `.prj`, UTM 32N is assumed, with a warning. Minna and the Nigerian belts are reprojected.
- **Placing** (*Place* window): click a beacon whose UTM 32N coordinates are printed on the plan and
  type them. Two points place the picture (one scale, a rotation, a shift; never stretched); from
  three on, each point's disagreement in metres is shown, and over 0.5 m is flagged. An EMF's own
  corners are snapped to.
- A TIFF or BMP is shown under the map's parcels (transparency slider, *Place again*, *Hide*) to
  trace with *Draw boundary*. It has no boundary of its own; nothing is read off it automatically.
- Corners traced from a picture, or of an EMF placed by points, are only as good as the picture.
  Type the plan's printed coordinates over them before relying on the check.

### Export (*Export…* in the panel)

The boundary and its corners, and if wanted the imported drawing. Corners have the beacon numbers
that came with typed points, else P1, P2, ….

| File | Content |
|---|---|
| DXF | R12 ASCII, UTM 32N (`POST /api/v1/export/drawing`, `cadastre-import/src/export.rs`) |
| DWG | that DXF through the ODA File Converter, AutoCAD 2018 (needs the converter, as import does) |
| KML | WGS 84 longitude / latitude |
| EMF | lines and labels on a 250 × 180 mm page |
| TIFF | a north-up picture as a GeoTIFF (EPSG:32632), with the scanned plan under the lines if wanted |
| BMP | the same picture, with its `.bpw` and `.prj` (the browser may ask to allow several downloads) |

Tests: `kml`, `emf`, `export` and `picture` unit tests, `drawings_export_and_import_back`
(integration) and `the_real_converter_reads_a_dwg` (DXF to DWG and back). Checked by hand in a
browser (24 Nov 2026): every format exported and read back, a scan and an EMF placed by points. Not
yet tried: surveyors' own files, and the exported files in AutoCAD, Google Earth and Word.
