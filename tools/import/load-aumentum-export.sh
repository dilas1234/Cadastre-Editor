#!/usr/bin/env bash
# Load an Aumentum export (tools/aumentum-inventory/aumentum-export.ps1 output) into the import
# staging schema as ONE import run, then verify that every exported row is accounted for.
# It does not promote: review the run, then POST /api/v1/import/promote/<run>.
#
#   tools/import/load-aumentum-export.sh <export-dir>
#
# Env:
#   CADASTRE_IMPORT_PG  target PostGIS (default postgres://cadastre:cadastre@localhost:5432/cadastre)
#   PSQL                psql command (default: psql inside the cadastre-db container)
#   IMPORT_FILE         import-file binary (default: built with cargo --release if missing)
set -euo pipefail
DIR="${1:?usage: load-aumentum-export.sh <export-dir>}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export CADASTRE_IMPORT_PG="${CADASTRE_IMPORT_PG:-postgres://cadastre:cadastre@localhost:5432/cadastre}"
PSQL="${PSQL:-docker exec -i cadastre-db psql -U cadastre -d cadastre -v ON_ERROR_STOP=1 -q -At}"
for f in parcels.psv beacons.psv parcel-attributes.psv upins-issued.psv; do
  [ -s "$DIR/$f" ] || { echo "missing or empty: $DIR/$f" >&2; exit 1; }
done

IMPORT_FILE="${IMPORT_FILE:-$ROOT/target/release/import-file}"
if [ ! -x "$IMPORT_FILE" ]; then
  echo "building import-file ..."
  (cd "$ROOT" && cargo build -q --release -p cadastre-import --features sink --bin import-file)
fi

echo "== parcels + beacons"
out="$("$IMPORT_FILE" "$DIR/parcels.psv" "$DIR/beacons.psv")"
echo "$out"
RUN="$(sed -n 's/^LOADED run \([0-9]*\):.*/\1/p' <<<"$out")"
[ -n "$RUN" ] || { echo "could not read the run id from import-file output" >&2; exit 1; }

# '|'-separated, header line, empty field = NULL. QUOTE is set to a control character so that
# quote marks inside Aumentum values are taken literally.
copy_opts="(FORMAT csv, DELIMITER '|', HEADER true, NULL '', QUOTE E'\\x01')"
# Print a file with Unix line endings (the export is written on Windows, CR+LF), adding a final
# newline only if it lacks one (a blank line would be read as a row), so the COPY end-of-data
# marker always starts its own line.
with_eol() { tr -d '\r' < "$1"; [ -z "$(tail -c1 "$1" | tr -d '\r')" ] || echo; }

echo "== parcel attributes (incl. Registry linkage) -> run $RUN"
{
  cat <<SQL
CREATE TEMP TABLE t (
  OBJECTID text, DatasetName text, RegionName text, ParcelID text, UPIN text, ParcelName text, Parent text,
  PropertyNumber text, PropertyType text, Plot text, Block text, Location text, ParcelArea text, AreaUnit text,
  PlanNumber text, LGA text, Status text, CanRollback text, IsStrata text, MutationType text, CreationDate text,
  LastEditDate text, ConsolidationDate text, ArchiveReason text, ArchiveDate text, LRSID text, LinkageStatus text,
  LinkageDate text, SurveyorGeneral text);
COPY t FROM STDIN $copy_opts;
SQL
  with_eol "$DIR/parcel-attributes.psv"; printf '\\.\n'
  cat <<SQL
INSERT INTO import.parcel_attribute (run_id, object_id, dataset_name, region_name, parcel_id, upin, parcel_name,
  parent, property_number, property_type, plot, block, location, parcel_area, area_unit, plan_number, lga, status,
  can_rollback, is_strata, mutation_type, creation_date, last_edit_date, consolidation_date, archive_reason,
  archive_date, lrs_id, linkage_status, linkage_date, surveyor_general)
SELECT $RUN, OBJECTID::int, DatasetName, RegionName, ParcelID, UPIN, ParcelName, Parent, PropertyNumber, PropertyType,
  Plot, Block, Location, ParcelArea, AreaUnit, PlanNumber, LGA, Status, CanRollback, IsStrata, MutationType,
  CreationDate, LastEditDate, ConsolidationDate, ArchiveReason, ArchiveDate, LRSID, LinkageStatus, LinkageDate,
  SurveyorGeneral FROM t;
SELECT 'attributes staged: ' || count(*) FROM import.parcel_attribute WHERE run_id = $RUN;
SQL
} | $PSQL

echo "== every issued UPIN -> run $RUN"
{
  printf 'CREATE TEMP TABLE u (upin text, first_source text, occurrences text);\nCOPY u FROM STDIN %s;\n' "$copy_opts"
  with_eol "$DIR/upins-issued.psv"; printf '\\.\n'
  cat <<SQL
INSERT INTO import.issued_upin (run_id, upin, first_source, occurrences)
SELECT $RUN, upin, first_source, NULLIF(occurrences, '')::int FROM u WHERE upin IS NOT NULL;
SELECT 'issued UPINs staged: ' || count(*) FROM import.issued_upin WHERE run_id = $RUN;
SQL
} | $PSQL

echo "== reconciliation of run $RUN against the export files"
lines() { grep -cv '^[[:space:]]*$' "$1"; }
P=$(lines "$DIR/parcels.psv"); B=$(lines "$DIR/beacons.psv")
A=$(( $(lines "$DIR/parcel-attributes.psv") - 1 )); U=$(( $(lines "$DIR/upins-issued.psv") - 1 ))
$PSQL <<SQL
SELECT format('parcels:    file %s | staged %s + exceptions %s = %s  %s', $P, s, e, s + e, CASE WHEN s + e = $P THEN 'OK' ELSE 'MISMATCH' END)
  FROM (SELECT (SELECT count(*) FROM import.parcel WHERE run_id = $RUN) s, (SELECT count(*) FROM import.exception WHERE run_id = $RUN) e) x;
SELECT format('beacons:    file %s | staged %s  %s', $B, count(*), CASE WHEN count(*) = $B THEN 'OK' ELSE 'MISMATCH' END) FROM import.beacon WHERE run_id = $RUN;
SELECT format('attributes: file %s | staged %s  %s', $A, count(*), CASE WHEN count(*) = $A THEN 'OK' ELSE 'MISMATCH' END) FROM import.parcel_attribute WHERE run_id = $RUN;
SELECT format('UPINs:      file %s | staged %s  %s', $U, count(*), CASE WHEN count(*) = $U THEN 'OK' ELSE 'MISMATCH' END) FROM import.issued_upin WHERE run_id = $RUN;
SELECT format('  status %-9s %s', status, count(*)) FROM import.parcel WHERE run_id = $RUN GROUP BY status ORDER BY status;
SELECT format('  exception %-20s %s', kind, count(*)) FROM import.exception WHERE run_id = $RUN GROUP BY kind ORDER BY kind;
SQL
echo "RUN=$RUN"
