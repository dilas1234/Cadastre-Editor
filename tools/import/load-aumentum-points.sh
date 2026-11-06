#!/usr/bin/env bash
# Load Aumentum's survey points (points.psv from aumentum-export.ps1, Plateau_State_Points in the
# DEFAULT version) into cadastre.beacon as survey beacons (is_control = false). Re-runnable: a
# beacon already in the editor is kept as it is (never moved), nothing is duplicated.
#
#   tools/import/load-aumentum-points.sh <export-dir>/points.psv
#
# Every exported row is accounted for:
#   loaded       one beacon per point name
#   same beacon  further records of a name at the same place (within SAME_M, default 0.5 m: each
#                job stored its own copy of a shared pillar): one beacon kept, at the newest record
#   unnamed      fabric vertices with no name, or Aumentum's temporary labels P1, P2, ... (the
#                computation's point numbers, not beacon numbers); the parcels hold these corners
#   clash        a name used at two different places, or a name that is a control point's at a
#                different place: NOT loaded; written to <points.psv dir>/points-clashes.psv for
#                PLAGIS to resolve
# Env: PSQL (default: psql inside the cadastre-db container, database cadastre), SAME_M
set -euo pipefail
FILE="${1:?usage: load-aumentum-points.sh <points.psv>}"
[ -s "$FILE" ] || { echo "missing or empty: $FILE" >&2; exit 1; }
PSQL="${PSQL:-docker exec -i cadastre-db psql -U cadastre -d cadastre -v ON_ERROR_STOP=1 -q -At}"
copy_opts="(FORMAT csv, DELIMITER '|', HEADER true, NULL '', QUOTE E'\\x01')"
with_eol() { tr -d '\r' < "$1"; [ -z "$(tail -c1 "$1" | tr -d '\r')" ] || echo; }
ROWS=$(( $(grep -cv '^[[:space:]]*$' "$FILE") - 1 ))
SAME_M="${SAME_M:-0.5}"
[[ "$SAME_M" =~ ^[0-9]+(\.[0-9]+)?$ ]] || { echo "SAME_M must be a number of metres" >&2; exit 1; }
CLASHES="$(dirname "$FILE")/points-clashes.psv"

{
  cat <<SQL
BEGIN;
CREATE TEMP TABLE t (OBJECTID text, PointID text, PointName text, Category text, Type text, Units text, Weight text,
  ControlPointID text, JobID text, TempID text, X text, Y text, Z text, ShapeX text, ShapeY text);
COPY t FROM STDIN $copy_opts;
SQL
  with_eol "$FILE"; printf '\\.\n'
  cat <<SQL
CREATE TEMP TABLE p AS
SELECT OBJECTID::int AS oid, CASE WHEN btrim(PointName) ~ '^P[0-9]+$' THEN NULL ELSE NULLIF(btrim(PointName), '') END AS name,
       btrim(PointName) AS raw_name, JobID AS job,
       ST_SetSRID(ST_MakePoint(COALESCE(X, ShapeX)::float8, COALESCE(Y, ShapeY)::float8), 32632) AS geom
FROM t;

-- The position of a name is its newest record's (highest OBJECTID); a name is a clash if any of
-- its records lies more than SAME_M from that, or if a control point has the name elsewhere.
CREATE TEMP TABLE named AS
SELECT name, (array_agg(geom ORDER BY oid DESC))[1] AS geom, count(*) AS records, array_agg(oid ORDER BY oid) AS oids
FROM p WHERE name IS NOT NULL GROUP BY name;
CREATE INDEX ON p (name);
CREATE TEMP TABLE clash AS
SELECT n.name, n.oids, CASE WHEN c.id IS NOT NULL THEN 'control point ' || c.number || ' is elsewhere' ELSE 'used at different places' END AS why,
       (SELECT round(max(ST_Distance(q.geom, n.geom))::numeric, 2) FROM p q WHERE q.name = n.name) AS spread_m
FROM named n
LEFT JOIN cadastre.beacon c ON c.number = n.name AND c.is_control AND ST_Distance(c.geom, n.geom) > $SAME_M
WHERE c.id IS NOT NULL OR EXISTS (SELECT 1 FROM p WHERE p.name = n.name AND ST_Distance(p.geom, n.geom) > $SAME_M);

-- A name that is already a control point at the same place stays the control point.
CREATE TEMP TABLE is_ctrl AS
SELECT n.name FROM named n JOIN cadastre.beacon c ON c.number = n.name AND c.is_control
WHERE n.name NOT IN (SELECT name FROM clash);

INSERT INTO cadastre.beacon (number, geom, is_control)
SELECT name, geom, false FROM named WHERE name NOT IN (SELECT name FROM clash) AND name NOT IN (SELECT name FROM is_ctrl)
ON CONFLICT (number, duplicate_seq) DO NOTHING; -- a beacon already in the editor (an earlier load, or committed since) is never moved
SELECT format('  already in the editor (kept):  %s', count(*)) FROM named n JOIN cadastre.beacon b ON b.number = n.name
  WHERE NOT b.is_control AND n.name NOT IN (SELECT name FROM clash) AND NOT ST_DWithin(b.geom, n.geom, $SAME_M);

SELECT format('points.psv rows:            %s', count(*)) FROM p;
SELECT format('  survey beacons (one per name): %s', count(*)) FROM named WHERE name NOT IN (SELECT name FROM clash) AND name NOT IN (SELECT name FROM is_ctrl);
SELECT format('  same place as a control point: %s', count(*)) FROM is_ctrl;
SELECT format('  same beacon recorded again:    %s rows', COALESCE(sum(records - 1), 0)) FROM named WHERE name NOT IN (SELECT name FROM clash);
SELECT format('  unnamed / temporary P-labels:  %s rows (not beacons)', count(*)) FROM p WHERE name IS NULL;
SELECT format('  name clashes, NOT loaded:      %s names, %s rows', count(*), COALESCE(sum(cardinality(oids)), 0)) FROM clash;
SELECT format('  accounted for:                 %s of %s rows  %s',
  a, $ROWS, CASE WHEN a = $ROWS THEN 'OK' ELSE 'MISMATCH' END)
FROM (SELECT (SELECT count(*) FROM p WHERE name IS NULL)
           + (SELECT COALESCE(sum(records), 0) FROM named) AS a) s;
SELECT format('cadastre.beacon now:        %s survey beacons, %s control points',
  count(*) FILTER (WHERE NOT is_control), count(*) FILTER (WHERE is_control)) FROM cadastre.beacon;
SELECT '--- first name clashes (name: why, spread, Aumentum OBJECTIDs); all are in points-clashes.psv ---' WHERE EXISTS (SELECT 1 FROM clash);
SELECT format('  %s: %s, %s m apart, OBJECTID %s', name, why, spread_m, array_to_string(oids, ', ')) FROM clash ORDER BY name LIMIT 20;
SELECT '@@CLASHES@@';
SELECT format('%s|%s|%s|%s|%s|%s|%s', c.name, c.why, q.oid, q.job, round(ST_X(q.geom)::numeric, 3), round(ST_Y(q.geom)::numeric, 3), c.spread_m)
FROM clash c JOIN p q ON q.name = c.name ORDER BY c.name, q.oid;
COMMIT;
SQL
} | $PSQL | {
  while IFS= read -r line; do [ "$line" = "@@CLASHES@@" ] && break; printf '%s\n' "$line"; done
  { echo "name|why|aumentum_objectid|job_id|easting|northing|spread_m"; cat; } > "$CLASHES"
}
echo "name clashes written to $CLASHES ($(( $(wc -l < "$CLASHES") - 1 )) records)"
