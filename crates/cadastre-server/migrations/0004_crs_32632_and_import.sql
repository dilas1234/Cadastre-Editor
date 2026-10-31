-- Align the live fabric to its real CRS and add the import staging schema.
--
-- The production Aumentum fabric is stored in EPSG:32632 (WGS84 / UTM zone 32N), not the Minna
-- 26392 originally assumed, so the live geometry columns switch to 32632. ST_SetSRID only relabels
-- (dev tables hold synthetic test rows); production starts empty.
ALTER TABLE cadastre.beacon
    ALTER COLUMN geom TYPE geometry(Point, 32632) USING ST_SetSRID(geom, 32632);
-- Parcels accept Polygon and MultiPolygon (a few imported parcels are multipolygons).
ALTER TABLE cadastre.parcel
    ALTER COLUMN geom TYPE geometry(Geometry, 32632) USING ST_SetSRID(geom, 32632);

-- Staging schema written by cadastre-import (mirrors its Sink::ensure_schema). Defined here too
-- so the server's promotion endpoint and tests have it even before the importer runs.
CREATE SCHEMA IF NOT EXISTS import;

CREATE TABLE IF NOT EXISTS import.run (
    id bigserial PRIMARY KEY,
    started_at timestamptz NOT NULL DEFAULT now(),
    parcels integer NOT NULL DEFAULT 0,
    beacons integer NOT NULL DEFAULT 0,
    exceptions integer NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS import.parcel (
    id bigserial PRIMARY KEY,
    run_id bigint NOT NULL REFERENCES import.run (id) ON DELETE CASCADE,
    source_object_id integer NOT NULL,
    parcel_number text,
    status text NOT NULL,
    job_id text, plan_no text, cad_zone text, plot_no text, block_no text,
    computed_area double precision,
    srid integer NOT NULL,
    geom geometry(Geometry, 32632) NOT NULL
);
CREATE INDEX IF NOT EXISTS import_parcel_run_idx ON import.parcel (run_id);

CREATE TABLE IF NOT EXISTS import.beacon (
    id bigserial PRIMARY KEY,
    run_id bigint NOT NULL REFERENCES import.run (id) ON DELETE CASCADE,
    number text NOT NULL,
    geom geometry(Point, 32632) NOT NULL
);

CREATE TABLE IF NOT EXISTS import.exception (
    id bigserial PRIMARY KEY,
    run_id bigint NOT NULL REFERENCES import.run (id) ON DELETE CASCADE,
    source_object_id integer,
    parcel_number text,
    kind text NOT NULL
);
