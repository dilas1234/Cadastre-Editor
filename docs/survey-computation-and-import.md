# Survey computation, import, topology and adjustment

This feature branch implements the survey-computation epic: the 35 COGO/curve/area/geodetic
tools, CAD and feature-class import, attribute import, raster world files, topology review, and
alignment/conflation. Every result comes from `cadastre-core`; the editor, the REST API and the
golden-file tests run the same code.

> **Golden-file status.** `crates/cadastre-core/tests/fixtures/golden/` and
> `crates/cadastre-import/tests/fixtures/golden/` currently hold **synthetic regression
> fixtures** (each file's `source` field says so). They prove the two systems agree with each
> other and with independently computed values. To satisfy the epic exit criterion, replace them
> with the tabulations from real survey plans; the harnesses need no changes.

## The 35 computation tools

`cadastre_core::tools::catalog()` is the authoritative registry. The tools are grouped as:

| Category | Count | Tools |
| --- | ---: | --- |
| Basic COGO | 7 | `inverse`, `forward`, `traverse`, `angle-convert`, `distance-convert`, `point-on-line`, `perpendicular-foot` |
| Intersections | 5 | `intersect-bearings`, `intersect-lines`, `intersect-bearing-distance`, `intersect-distances`, `offset-intersection` |
| Curves | 6 | `curve-radius-delta`, `curve-components`, `curve-three-points`, `curve-tangents`, `curve-point-at`, `spiral` |
| Area and adjustment | 5 | `area`, `closure`, `bowditch`, `least-squares`, `area-check` |
| Transformations | 7 | `rotate`, `translate`, `scale`, `helmert-fit`, `helmert-apply`, `conflate-points`, `conflate-lines` |
| Geodetic | 3 | `project`, `unproject`, `grid-scale-factor` |
| Line utilities | 2 | `line-divide`, `offset-point` |

Bearing inputs are whole-circle **decimal degrees**; coordinates are easting/northing metres in
EPSG:32632. `curve-degree` and `midpoint` are not separate tools: degree-of-curve output comes
from `curve-components`, and midpoint division comes from `line-divide`.

### REST surface

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/v1/survey/tools` | Catalogue: ids, categories, inputs and result shape |
| `GET` | `/api/v1/survey/tools/{id}` | One tool specification |
| `POST` | `/api/v1/survey/tools/{id}` | Execute with a JSON input object |

The response is `{ "tool": …, "name": …, "result": … }`. Unknown ids return 404; malformed or
incomplete input returns 400.

In the editor these are under **Geoprocessing → Computation Tools (35)…**. The tool window builds
its inputs from the catalogue (point pickers can read the current map centre) and displays the
raw JSON result, so adding a core tool does not require a parallel JavaScript implementation.

## Import

`cadastre_import::conversion::import_text()` detects the format from the file name and, where
useful, the contents. All parsers are dependency-free so the crate still builds for the Windows 7
target and for wasm.

| Input | Behaviour |
| --- | --- |
| CSV / TSV / whitespace lists | Header-aware coordinate lists; point id, easting, northing, elevation and code columns are recognised. Bad rows are reported as line issues instead of being dropped silently. |
| ASCII DXF | `POINT`, `LINE`, `LWPOLYLINE`, `POLYLINE` and `TEXT`/`MTEXT` labels. Closed polylines become boundaries; blocks (`INSERT`) are reported, not exploded. Binary DXF is not supported. |
| LandXML | `CgPoints` and `Parcels/CoordGeom` lines, arcs and spirals. Curves are tessellated to chords no longer than 0.25 m. LandXML coordinates are **northing easting [elevation]**, converted to easting/northing. |
| World files (`.tfw`, `.jgw`, `.pgw`, …) | Pixel-to-map affine transform plus an optional sibling `.prj`. Sidecar files only georeference raster data; they do not create vector geometry. |
| Attribute table | Joined onto a coordinate list by point id (`/import/attributes` or `key_column`). Unjoined points are warned about, never silently invented. |

Import never reprojects. A file that names a CRS other than EPSG:32632 is stored as-is and the
job import returns a warning telling the operator to reproject first. `MAX_IMPORT_BYTES` is
16 MiB.

### REST surface

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/v1/import/preview` | Parse and describe any supported file, writing nothing |
| `POST` | `/api/v1/import/attributes` | Parse a coordinate list and join an attribute table |
| `POST` | `/api/v1/jobs/{id}/import` | Turn closed boundaries into draft parcels and points into beacons |

Job import only creates `draft` parcels/beacons; the normal submit → review → approve → post
workflow still applies. In the editor use **File → Import Survey / CAD / Attributes…**.

## Topology review and adjustment

`cadastre_core::topology::check_topology()` reports invalid rings, slivers, overlaps, duplicate
beacons, enclosed gaps, self-intersections and near-miss vertices. Rings may be supplied open or
closed — polygon semantics close them implicitly, which matches the area engine.

`POST /api/v1/fabric/validate` returns all of those findings plus `is_clean`. `POST
/api/v1/fabric/adjust` runs a weighted least-squares network adjustment and flags residuals
against the active tolerances. `cadastre_core::alignment` provides least-squares alignment and
conflation primitives used by the transformation tools.

## Tests

```bash
cargo test -p cadastre-core -p cadastre-import -p cadastre-server
```

DB-backed tests also run when a throwaway PostGIS database is provided:

```bash
CADASTRE_TEST_DATABASE_URL='postgres://cadastre:cadastre@localhost:5432/cadastre' \
  cargo test -p cadastre-core -p cadastre-import -p cadastre-server
```

Key harnesses:

- `crates/cadastre-core/tests/golden_surveys.rs` — vertices (1 mm), closure (1 mm), area
  (0.01 m²), perimeter, and the direct `area`/`closure` entry points.
- `crates/cadastre-import/tests/golden_import.rs` — CSV, DXF and LandXML agree on the same parcel;
  clean input is silent; world files georeference rasters.
- `crates/cadastre-server/tests/integration.rs` — catalogue/execution, previews for every format,
  attribute joins and job import.

## Known limitations / follow-ups

- Real-plan golden fixtures are still required for the epic exit criterion (see the status note
  at the top).
- DXF is ASCII-only and does not explode block references.
- Curves imported from LandXML are chorded at 0.25 m; this is an approximation, not the original
  arc definition.
- Imported geometry is assumed to be EPSG:32632; no reprojection is performed.
