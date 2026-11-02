#!/usr/bin/env bash
# Phase 5: build a COPY of the Registry database (LRS43) from the read-only Registry export
# (tools/aumentum-inventory/RUN-ME-REGISTRY-EXPORT.cmd), for developing and testing the
# Cadastre <-> Registry linkage. The live Registry is never written to.
#
#   tools/import/load-lrs-copy.sh <lrs-export>/registry
#
# Into the test SQL Server container (default cadastre-mssql-test), database LRS43_COPY, which is
# DROPPED AND REBUILT on every run (it is only ever this copy):
#   1. schema/lrs-tables.sql (every LRSAdmin table, empty)
#   2. data/*.psv.gz through bcp; columns the export left out (free text, bank details) stay NULL
#   3. schema/lrs-indexes.sql, lrs-fks.sql (WITH NOCHECK: the copy is partial by design),
#      lrs-objects.sql (views/functions; one that does not compile is reported, not fatal)
#   4. reconciliation: rows loaded per table against lrs-export-summary.txt
#
# Env: MSSQL_CONTAINER (cadastre-mssql-test), MSSQL_SA_PASSWORD (default: the container's own),
#      LRS_COPY_DB (LRS43_COPY)
set -euo pipefail
SRC="${1:?usage: load-lrs-copy.sh <lrs-export>/registry}"
[ -s "$SRC/schema/lrs-tables.sql" ] || { echo "not a Registry export (no schema/lrs-tables.sql): $SRC" >&2; exit 1; }
C="${MSSQL_CONTAINER:-cadastre-mssql-test}"
DB="${LRS_COPY_DB:-LRS43_COPY}"
[[ "$DB" =~ ^[A-Za-z0-9_]+$ ]] || { echo "bad database name: $DB" >&2; exit 1; }
[[ "$DB" != "LRS43" ]] || { echo "refusing to use the name LRS43 for the copy" >&2; exit 1; }
PW="${MSSQL_SA_PASSWORD:-$(docker inspect "$C" --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^MSSQL_SA_PASSWORD=//p')}"
T=/opt/mssql-tools18/bin
sq() { docker exec -i -e SQLCMDPASSWORD="$PW" "$C" $T/sqlcmd -C -S localhost -U sa -b -I "$@"; }
val() { sq -d "$DB" -h -1 -W -Q "SET NOCOUNT ON; $1" < /dev/null | tr -d '\r' | sed '/^$/d'; }
WORK=/tmp/lrs-copy-load
docker exec "$C" bash -c "rm -rf $WORK && mkdir -p $WORK"
trap 'docker exec "$C" rm -rf "$WORK" >/dev/null 2>&1 || true' EXIT

echo "1. structure -> $C / $DB (rebuilt)"
sq -Q "IF DB_ID('$DB') IS NOT NULL BEGIN ALTER DATABASE [$DB] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE [$DB]; END; CREATE DATABASE [$DB];" >/dev/null
docker cp "$SRC/schema/." "$C:$WORK/" >/dev/null
sq -d "$DB" -i "$WORK/lrs-tables.sql" >/dev/null
echo "   tables: $(val "SELECT COUNT(*) FROM sys.tables")"

echo "2. rows"
FS='~|~'
for gz in "$SRC"/data/*.psv.gz; do
  [ -e "$gz" ] || continue
  t=$(basename "$gz" .psv.gz)
  [[ "$t" =~ ^[A-Za-z0-9_]+$ ]] || { echo "   skipped odd file name: $gz"; continue; }
  docker exec -i "$C" bash -c "gzip -dc > $WORK/$t.dat" < "$gz"
  # bcp format file: one field per exported column (header row order), each bound to its column
  # in the table; the table's other columns are left NULL.
  header=$( (gzip -dc "$gz" 2>/dev/null || true) | head -c 1000000 | awk -v RS='~#~\n' 'NR==1{print; exit}' || true)
  IFS=$'\n' read -r -d '' -a names < <(printf '%s' "$header" | awk -v RS='~\\|~' '{print}' && printf '\0') || true
  ords=$(val "SELECT name + '=' + CAST(column_id AS varchar(10)) FROM sys.columns WHERE object_id = OBJECT_ID('LRSAdmin.$t')")
  {
    echo "14.0"; echo "${#names[@]}"
    i=0
    for n in "${names[@]}"; do
      i=$((i+1))
      ord=$(printf '%s\n' "$ords" | awk -F= -v n="$n" 'tolower($1)==tolower(n){print $2; exit}')
      [ -n "$ord" ] || { echo "column $n of $t is not in the copy's table" >&2; exit 1; }
      term="$FS"; [ "$i" -eq "${#names[@]}" ] && term='~#~\n'
      printf '%d SQLCHAR 0 0 "%s" %d %s ""\n' "$i" "$term" "$ord" "$n"
    done
  } | docker exec -i "$C" bash -c "cat > $WORK/$t.fmt"
  # NOT NULL columns (outside the primary key) take NULL for the load: a column the export left
  # out arrives empty, and so does an empty string ('' and NULL are both an empty field).
  present=$(for n in "${names[@]}"; do printf "'%s'," "${n//\'/\'\'}"; done); present="${present%,}"
  sq -d "$DB" -Q "SET NOCOUNT ON;
    IF OBJECT_ID('dbo._lrs_notnull') IS NULL CREATE TABLE dbo._lrs_notnull (tbl sysname, col sysname, typ nvarchar(200), is_text bit, exported bit);
    DECLARE @s nvarchar(max) = N'';
    INSERT dbo._lrs_notnull
    SELECT '$t', c.name,
      CASE WHEN ty.name IN ('nvarchar','nchar') THEN ty.name + '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')'
           WHEN ty.name IN ('varchar','char','varbinary','binary') THEN ty.name + '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')'
           WHEN ty.name IN ('decimal','numeric') THEN ty.name + '(' + CAST(c.precision AS varchar(5)) + ',' + CAST(c.scale AS varchar(5)) + ')'
           WHEN ty.name IN ('datetime2','time','datetimeoffset') THEN ty.name + '(' + CAST(c.scale AS varchar(5)) + ')'
           ELSE ty.name END,
      CASE WHEN ty.name IN ('nvarchar','nchar','varchar','char','text','ntext') THEN 1 ELSE 0 END,
      CASE WHEN c.name IN ($present) THEN 1 ELSE 0 END
    FROM sys.columns c JOIN sys.types ty ON ty.user_type_id = c.user_type_id
    WHERE c.object_id = OBJECT_ID('LRSAdmin.$t') AND c.is_nullable = 0 AND c.is_identity = 0 AND c.is_computed = 0
      AND NOT EXISTS (SELECT 1 FROM sys.index_columns ic JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id
                      WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id);
    SELECT @s += N'ALTER TABLE LRSAdmin.' + QUOTENAME(tbl) + N' ALTER COLUMN ' + QUOTENAME(col) + N' ' + typ + N' NULL; ' FROM dbo._lrs_notnull WHERE tbl = '$t';
    EXEC (@s);" < /dev/null >/dev/null
  out=$(docker exec -e P="$PW" "$C" bash -c "$T/bcp LRSAdmin.$t in $WORK/$t.dat -S localhost -U sa -P \"\$P\" -d $DB -u -f $WORK/$t.fmt -F 2 -k -E -C 65001 -b 20000 -e $WORK/$t.err 2>&1; echo; cat $WORK/$t.err 2>/dev/null | head -5") || true
  # Back to NOT NULL: an exported text column that was NOT NULL can only have held '' where the
  # field is empty; a column the export left out stays nullable (its values are not in the copy).
  sq -d "$DB" -Q "SET NOCOUNT ON; DECLARE @s nvarchar(max) = N'';
    SELECT @s += N'UPDATE LRSAdmin.' + QUOTENAME(tbl) + N' SET ' + QUOTENAME(col) + N' = '''' WHERE ' + QUOTENAME(col) + N' IS NULL; '
      FROM dbo._lrs_notnull WHERE tbl = '$t' AND is_text = 1 AND exported = 1;
    EXEC (@s); SET @s = N'';
    SELECT @s += N'IF NOT EXISTS (SELECT 1 FROM LRSAdmin.' + QUOTENAME(tbl) + N' WHERE ' + QUOTENAME(col) + N' IS NULL) ALTER TABLE LRSAdmin.' + QUOTENAME(tbl) + N' ALTER COLUMN ' + QUOTENAME(col) + N' ' + typ + N' NOT NULL; '
      FROM dbo._lrs_notnull WHERE tbl = '$t' AND exported = 1;
    EXEC (@s);" < /dev/null >/dev/null
  printf '   %-28s %s\n' "$t" "$(printf '%s\n' "$out" | grep -Eo '[0-9]+ rows copied' || echo "FAILED: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-300)")"
  docker exec "$C" rm -f "$WORK/$t.dat"
done

echo "3. indexes, foreign keys, views"
for f in lrs-indexes.sql lrs-fks.sql; do sq -d "$DB" -i "$WORK/$f" >/dev/null && echo "   $f applied"; done
if ! sq -d "$DB" -i "$WORK/lrs-objects.sql" >/dev/null 2>"/dev/null"; then
  docker exec -e SQLCMDPASSWORD="$PW" "$C" $T/sqlcmd -C -S localhost -U sa -I -d "$DB" -i "$WORK/lrs-objects.sql" 2>&1 | grep -E '^Msg' | sort | uniq -c | sed 's/^/   object not created: /' | head -10
else echo "   lrs-objects.sql applied"; fi

echo "4. reconciliation (rows exported -> rows in the copy)"
bad=0
while IFS='|' read -r t exported _; do
  [[ "$t" =~ ^lr_[A-Za-z0-9_]+$ ]] || continue
  n=$(val "SELECT COUNT_BIG(*) FROM LRSAdmin.$t")
  ok=OK; [ "$n" = "$exported" ] || { ok=MISMATCH; bad=1; }
  printf '   %-28s %9s -> %9s  %s\n' "$t" "$exported" "$n" "$ok"
done < <(tr -d '\r' < "$SRC/lrs-export-summary.txt")
sq -d "$DB" -Q "SET NOCOUNT ON; SELECT tbl + '.' + col AS left_nullable_not_exported FROM dbo._lrs_notnull WHERE exported = 0 ORDER BY 1" -W -h -1 < /dev/null | tr -d '\r' | sed '/^$/d; s/^/   nullable in the copy (not exported): /'
[ "$bad" -eq 0 ] && echo "LRS copy ready: $C / $DB" || { echo "LRS copy has mismatches (see above)" >&2; exit 1; }
