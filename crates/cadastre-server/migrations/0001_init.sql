-- Phase 2 initial schema for the PLAGIS Cadastre Editor.
--
-- All coordinates are stored in EPSG:26392 (Minna / Nigeria Mid Belt), the working
-- projected CRS used throughout cadastre-core (SRS §6). PostGIS ships this SRID in
-- spatial_ref_sys, so geometries are constrained to it directly.

CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE SCHEMA IF NOT EXISTS cadastre;

-- ---------------------------------------------------------------------------
-- Configurable tolerances (FR-ADM-03). Mirrors cadastre_core::tolerances::Tolerances.
-- A single active row; historical rows kept for audit (effective_from ordering).
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.tolerance_config (
    id                   uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    closure_precision    double precision NOT NULL,  -- min precision-ratio denominator (1:N)
    area_fraction        double precision NOT NULL,  -- max fractional area difference
    join_distance        double precision NOT NULL,  -- snap/join search distance (m)
    sliver_area          double precision NOT NULL,  -- min polygon/gap area (m²)
    duplicate_beacon     double precision NOT NULL,  -- duplicate-beacon distance (m)
    distance_residual    double precision NOT NULL,  -- max post-adjustment distance residual (m)
    bearing_residual_sec double precision NOT NULL,  -- max post-adjustment bearing residual (arcsec)
    is_active            boolean NOT NULL DEFAULT true,
    updated_by           text NOT NULL,
    updated_at           timestamptz NOT NULL DEFAULT now()
);

-- Only one active tolerance set at a time.
CREATE UNIQUE INDEX tolerance_config_single_active
    ON cadastre.tolerance_config (is_active)
    WHERE is_active;

-- Seed the provisional defaults from cadastre_core::tolerances::Tolerances::default().
INSERT INTO cadastre.tolerance_config (
    closure_precision, area_fraction, join_distance, sliver_area,
    duplicate_beacon, distance_residual, bearing_residual_sec, updated_by
) VALUES (
    10000.0, 0.01, 0.5, 1.0, 0.01, 0.05, 30.0, 'system:migration'
);

-- ---------------------------------------------------------------------------
-- Beacons: physical survey marks at parcel corners (SRS §1.3, §6).
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.beacon (
    id         uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    number     text NOT NULL UNIQUE,
    geom       geometry(Point, 26392) NOT NULL,
    accuracy   double precision NOT NULL DEFAULT 0.05,  -- a-priori 1σ accuracy (m)
    is_control boolean NOT NULL DEFAULT false,          -- held fixed in adjustment
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX beacon_geom_gix ON cadastre.beacon USING gist (geom);

-- ---------------------------------------------------------------------------
-- Parcels: closed boundary polygons with stated vs computed area (FR-COG-03).
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.parcel (
    id            uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    parcel_number text NOT NULL UNIQUE,
    geom          geometry(Polygon, 26392) NOT NULL,
    stated_area   double precision,  -- area on the survey plan (m²), if known
    computed_area double precision,  -- area from geometry (m²)
    status        text NOT NULL DEFAULT 'draft',  -- draft | submitted | posted
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX parcel_geom_gix ON cadastre.parcel USING gist (geom);

-- Ordered parcel corners → beacons (the boundary beacon sequence).
CREATE TABLE cadastre.parcel_corner (
    parcel_id  uuid NOT NULL REFERENCES cadastre.parcel (id) ON DELETE CASCADE,
    seq        integer NOT NULL,
    beacon_id  uuid NOT NULL REFERENCES cadastre.beacon (id) ON DELETE RESTRICT,
    PRIMARY KEY (parcel_id, seq)
);

-- ---------------------------------------------------------------------------
-- Jobs: survey capture/maintenance units (placeholder, fleshed out in Phase 3).
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.job (
    id         uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    reference  text NOT NULL UNIQUE,
    job_type   text NOT NULL,                 -- capture | maintenance | subdivision | ...
    status     text NOT NULL DEFAULT 'open',  -- workflow stage (configurable, Phase 3)
    created_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- Hash-chained audit log (NFR-07/11). The chaining logic lives in the app
-- (Phase 6); this table only stores the entries and their links.
-- ---------------------------------------------------------------------------
CREATE TABLE cadastre.audit_log (
    id          bigserial PRIMARY KEY,
    actor       text NOT NULL,
    action      text NOT NULL,
    entity      text NOT NULL,
    entity_id   text,
    payload     jsonb NOT NULL DEFAULT '{}'::jsonb,
    prev_hash   text,
    hash        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_log_entity_idx ON cadastre.audit_log (entity, entity_id);
