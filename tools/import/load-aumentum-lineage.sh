#!/usr/bin/env bash
# Phase 4: load Aumentum's parcel history into the fabric. Re-runnable; nothing is duplicated.
#   1. Retired parcels (Aumentum Status 3 historic, 4 archived) from the latest import run, as
#      history shapes under their UPINs (the active parcel of a UPIN is untouched).
#   2. Aumentum's lineage register (MCServer dbo.parcelLineage) into cadastre.parcel_lineage,
#      each ParcelID resolved to its UPIN through the parcel attribute table.
#
#   tools/import/load-aumentum-lineage.sh <aumentum-export>/mcserver/dbo.parcelLineage.psv
#
# Env: PSQL (default: psql inside the cadastre-db container, database cadastre)
set -euo pipefail
FILE="${1:?usage: load-aumentum-lineage.sh <path to dbo.parcelLineage.psv>}"
[ -s "$FILE" ] || { echo "missing or empty: $FILE" >&2; exit 1; }
PSQL="${PSQL:-docker exec -i cadastre-db psql -U cadastre -d cadastre -v ON_ERROR_STOP=1 -q -At}"
copy_opts="(FORMAT csv, DELIMITER '|', HEADER true, NULL '', QUOTE E'\\x01')"
with_eol() { tr -d '\r' < "$1"; [ -z "$(tail -c1 "$1" | tr -d '\r')" ] || echo; }
ROWS=$(( $(grep -cv '^[[:space:]]*$' "$FILE") - 1 ))

{
  cat <<SQL
BEGIN;
-- 1. Retired parcels from the latest import run.
CREATE TEMP TABLE run AS SELECT max(run_id) AS id FROM import.parcel;
CREATE TEMP TABLE retired AS
SELECT ip.parcel_number, ip.status, ip.geom, ip.computed_area FROM import.parcel ip, run
WHERE ip.run_id = run.id AND ip.status IN ('historic', 'archived') AND ip.parcel_number IS NOT NULL;
INSERT INTO cadastre.parcel (parcel_number, geom, computed_area, status)
SELECT r.parcel_number, r.geom, r.computed_area, r.status FROM retired r
WHERE NOT EXISTS (SELECT 1 FROM cadastre.parcel p WHERE p.parcel_number = r.parcel_number AND p.status = r.status AND ST_Equals(p.geom, r.geom));
SELECT format('retired parcels in the import run:  %s (historic %s, archived %s)', count(*), count(*) FILTER (WHERE status = 'historic'), count(*) FILTER (WHERE status = 'archived')) FROM retired;
SELECT format('retired parcels now in the fabric:  %s  %s', count(*), CASE WHEN count(*) >= (SELECT count(*) FROM retired) THEN 'OK' ELSE 'MISMATCH' END)
FROM cadastre.parcel p WHERE p.status IN ('historic', 'archived');
SELECT format('active parcels (unchanged):         %s', count(*)) FROM cadastre.parcel WHERE status = 'posted';

-- 2. The lineage register.
CREATE TEMP TABLE l (id text, parcel text, status text, parent text, act text, hist text, mid text, mtype text);
COPY l FROM STDIN $copy_opts;
SQL
  with_eol "$FILE"; printf '\\.\n'
  cat <<SQL
-- A ParcelID's UPIN: its attribute rows, the active one first.
CREATE TEMP TABLE pu AS
SELECT DISTINCT ON (parcel_id) parcel_id, upin FROM cadastre.parcel_attribute WHERE parcel_id IS NOT NULL
ORDER BY parcel_id, (status = '0') DESC NULLS LAST, last_edit_date DESC NULLS LAST, object_id DESC;
INSERT INTO cadastre.parcel_lineage (parcel_id, upin, status, parent_id, parent_upin, changed_to_active, changed_to_history,
                                     mutation_id, mutation_type, source, aumentum_id)
SELECT l.parcel::uuid, c.upin, l.status::smallint, l.parent::uuid, p.upin, l.act::timestamp, l.hist::timestamp,
       l.mid, l.mtype::smallint, 'aumentum', l.id::uuid
FROM l LEFT JOIN pu c ON c.parcel_id = l.parcel::uuid LEFT JOIN pu p ON p.parcel_id = l.parent::uuid
ON CONFLICT (aumentum_id) DO NOTHING;
SELECT format('lineage rows in the file:           %s', count(*)) FROM l;
SELECT format('lineage rows loaded (aumentum):     %s  %s', count(*), CASE WHEN count(*) = $ROWS THEN 'OK' ELSE 'MISMATCH' END)
FROM cadastre.parcel_lineage WHERE source = 'aumentum';
SELECT format('  parcels resolved to a UPIN:       %s of %s', count(*) FILTER (WHERE upin IS NOT NULL), count(*)) FROM cadastre.parcel_lineage WHERE source = 'aumentum';
SELECT format('  parents resolved to a UPIN:       %s of %s', count(*) FILTER (WHERE parent_upin IS NOT NULL), count(*) FILTER (WHERE parent_id IS NOT NULL)) FROM cadastre.parcel_lineage WHERE source = 'aumentum';
SELECT format('  mutations by type:                %s', string_agg(t || ' x' || n, ', ' ORDER BY t)) FROM
  (SELECT mutation_type::text AS t, count(DISTINCT mutation_id) AS n FROM cadastre.parcel_lineage WHERE source = 'aumentum' AND mutation_id IS NOT NULL GROUP BY 1) s;
COMMIT;
SQL
} | $PSQL
