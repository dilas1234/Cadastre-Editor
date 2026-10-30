# Aumentum → Cadastre import (Phase 8 design)

Derived from a read-only inspection of the live PLAGIS Aumentum production server on 2026-10-30.
This is the contract the `cadastre-import` crate implements.

## Source system

- **Host** `10.10.10.6` (`PRODUCTIONMC`), Windows Server 2012 R2, SQL Server instance
  `.\PRODUCTIONMC`. Spatial data is in the **`sde` database (121 GB)**, an **Esri ArcSDE
  *versioned* geodatabase**. Registry/ownership attributes are in a separate **`LRS43`** database
  on `10.10.10.5` (not yet inspected).
- Geometry is stored in the **native SQL Server `geometry` type** (column `SHAPE`), so it is
  readable with `STAsText()`/`STAsBinary()` and needs no Esri libraries.
- **CRS = EPSG:32632 (WGS 84 / UTM zone 32N)** — confirmed from `SHAPE.STSrid` and the parcel
  shapefile `.prj`. This is now supported in `cadastre_core::crs` (`WorkingCrs::utm_zone_32n`).

## The versioning problem (critical)

Feature classes are **registered as multiversioned**: the business table holds only the base
state, while edits live in per-feature-class delta tables `a<id>` (adds) and `D<id>` (deletes),
where `<id>` is the `registration_id` from `sde.sde_table_registry`. **A plain `SELECT` on the
business table is wrong** — e.g. `PLATEAU_STATE_PARCELS` (registration_id 11) has only **286** base
rows, but the current **DEFAULT** version has **21,266** parcels (validated). Reconstruction is
mandatory.

Delta schema (confirmed):
- `a11` = all business columns + `SDE_STATE_ID bigint`.
- `D11` = `SDE_STATE_ID bigint`, `SDE_DELETES_ROW_ID int` (the deleted OBJECTID),
  `DELETED_AT bigint`.
- `sde.SDE_versions` maps a version name → `state_id` (DEFAULT → **1057484**).
- `sde.SDE_states` is the state tree; `sde.SDE_state_lineages(lineage_name, lineage_id)` lists all
  ancestor states of a state. DEFAULT's lineage has **14,993** states.

### Reconstruction query (DEFAULT version of feature class with registration_id R)

```sql
-- L = all states in the DEFAULT version's lineage
SELECT lineage_id INTO #L
FROM sde.SDE_state_lineages
WHERE lineage_name = (SELECT lineage_name FROM sde.SDE_states WHERE state_id = 1057484);

-- current rows = base (state 0) + adds in lineage, minus rows deleted in lineage
SELECT <cols> FROM sde.<business> b
  WHERE NOT EXISTS (SELECT 1 FROM sde.D<R> d
     WHERE d.SDE_DELETES_ROW_ID=b.OBJECTID AND d.SDE_STATE_ID=0
       AND d.DELETED_AT IN (SELECT lineage_id FROM #L))
UNION ALL
SELECT <cols> FROM sde.a<R> a
  WHERE a.SDE_STATE_ID IN (SELECT lineage_id FROM #L)
    AND NOT EXISTS (SELECT 1 FROM sde.D<R> d
     WHERE d.SDE_DELETES_ROW_ID=a.OBJECTID AND d.SDE_STATE_ID=a.SDE_STATE_ID
       AND d.DELETED_AT IN (SELECT lineage_id FROM #L));
```

(Materialise `#L` with a clustered index; the semijoins are then fast.) Equivalent options if
available on the box: create Esri **versioned views** (`sde.sde_create_versioned_view`) or export
the DEFAULT version with ArcGIS — but the SQL above needs neither and is what the importer uses.

## Feature classes → our domain

| Aumentum feature class (reg_id) | current rows | Maps to |
|---|---|---|
| `PLATEAU_STATE_PARCELS` (11) | 21,266 | `parcel` |
| `PLAGIS_COORDINATES` (155, **not versioned**, 164 rows) | 164 | control `beacon`s |
| `PLATEAU_STATE_POINTS` (9), `PLATEAU_STATE_LINES` (10) | — | fabric points/lines (later) |
| `PLATEAU_STATE_CTRLPNTS` (8) | — | control network |
| `PLATEAU_STATE_JOBS_*` (16–22) | — | per-job edit layers → our jobs (later) |

### `PLATEAU_STATE_PARCELS` columns → `parcel`

- `UPIN` (`PL` + 12 digits, e.g. `PL000000015533`) → `parcel_number`.
- `SHAPE` (geometry, SRID 32632) → `geom` (polygon; 21,213 Polygon, 3 MultiPolygon, 50 empty
  `Point`/placeholder rows to skip). Areas: min 8.5, avg ~4,468, max ~5.0M m².
- `Status` (smallint) → workflow/status. Observed: 2 = 18,899 (normal), 4 = 2,229, 1 = 49
  (empty placeholders), 3 = 85, plus 4 NULL. **Confirm the Status code meanings with PLAGIS.**
- `JobID` (uniqueidentifier) → link to the originating job.
- `PlanNo`/`CadZone`/`Layout`/`PlotNo`/`BlockNo`/`propertyNo` → addressing (sparse/NULL in base
  sample — likely populated per edit or in LRS43).
- `CreateDate`/`LastEditDate` present but often the zero date (1899-12-30) → treat as unknown.

### `PLAGIS_COORDINATES` columns → `beacon`

- `Alias` (e.g. `ABUZ`, `E2`, `N1`) → `beacon.number`; `X`,`Y` (numeric, UTM 32N) → point. These
  are the survey **control** points. Parcel-corner beacons are implicit in parcel polygon vertices.

## Import strategy (`cadastre-import`, Phase 8)

1. Connect to SQL Server with **tiberius** (pure-Rust, no C deps — protects NFR-02); read-only.
2. For each feature class, run the reconstruction query for the DEFAULT version and stream rows.
3. Parse `SHAPE` via `STAsBinary()` (WKB) or `STAsText()` (WKT) → `cadastre_core` geometry; keep
   SRID 32632.
4. Map to staging tables in PostGIS, then validate (topology, closure, duplicate UPIN, empty/placeholder
   geometry) and emit an **exceptions report**; only clean rows promote to the live schema.
5. Idempotent + re-runnable; never writes to the Aumentum database.

**Testing:** the pure parts (reconstruction SQL builder, row→domain mapping, WKB parsing, Status
mapping) are unit-tested with no DB. Live end-to-end runs target a **restored copy of the `sde`
database**, never the production server.

## Open items to confirm with PLAGIS
- Meaning of parcel `Status` codes (1–4) and `ModificationType`.
- Where authoritative owner/title attributes live (LRS43 vs parcel columns) and the UPIN↔title link.
- Whether unposted `Plateau_State_JobID_*` versions contain in-flight work to migrate as open jobs.
- Minna vs UTM 32N: live fabric is 32632; confirm whether any workflow still requires Minna output.
