<#
  READ-ONLY export of the live Aumentum cadastral data on PRODUCTIONMC, for the new Cadastre's
  no-loss import (Phase 1). Run inside the Remote Desktop session; output goes to
  <OutDir>\aumentum-export\ (i.e. back to the Linux machine via \\tsclient\inventory).

  Files (UTF-8, '|'-separated, empty field = NULL):
    parcels.psv            OBJECTID|UPIN|Status|JobID|PlanNo|CadZone|PlotNo|BlockNo|SRID|WKT
                           (no header; the format cadastre-import's import-file already reads)
    beacons.psv            Alias|X|Y   (no header): the PLAGIS_Coordinates control points
    points.psv             header line, then every survey point (beacon) of Plateau_State_Points in the
                           DEFAULT version: name, category, type, X/Y/Z, control-point and job links
    parcel-attributes.psv  header line, then every ParcelAttributeTable column, incl. the registry
                           link fields LRSID, LinkageStatus, LinkageDate
    upins-issued.psv       header line, then every UPIN found in ANY version or history state
                           (base + delta tables) of the parcels and the attribute table, so the new
                           system can reserve all of them, in-flight job versions included
    export-summary.txt     row counts per status, for reconciliation

  Current rows are the DEFAULT version, reconstructed as base + adds - deletes over DEFAULT's state
  lineage (same SQL as crates/cadastre-import/src/sde.rs). Only SELECT statements, READ
  UNCOMMITTED (never blocks editors); the only write on the server is the session's own #temp
  table. PowerShell 4 compatible.
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'), [string]$SqlInstance = '', [string]$SqlUser = '', [string]$SqlPassword = '',
      [ValidateSet('all', 'points')][string]$Only = 'all')
$ErrorActionPreference = 'Stop'
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where { $_.Name -notmatch 'WID|SQLEXPRESS|FDLauncher|OLAP' -and $_.Status -eq 'Running' } | Select -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
$dir = Join-Path $OutDir 'aumentum-export'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$inv = [Globalization.CultureInfo]::InvariantCulture
$utf8 = New-Object Text.UTF8Encoding($false)

function Clean($v) {
  if ($v -is [DBNull] -or $null -eq $v) { return '' }
  if ($v -is [double] -or $v -is [single]) { return $v.ToString('R', $inv) }
  if ($v -is [decimal]) { return $v.ToString($inv) }
  if ($v -is [datetime]) { return $v.ToString('yyyy-MM-ddTHH:mm:ss', $inv) }
  return ([string]$v) -replace '[\r\n|]+', ' '
}
# Stream one query's rows to a file; returns the row count.
function Export([string]$sql, [string]$file, [switch]$Header) {
  $cn = New-Object System.Data.SqlClient.SqlConnection ("Server=$SqlInstance;Database=sde;$(if ($SqlUser) { "User ID=$SqlUser;Password=$SqlPassword;TrustServerCertificate=True;" } else { 'Integrated Security=SSPI;' })Application Name=readonly-export;ApplicationIntent=ReadOnly;Connect Timeout=15")
  $cn.Open()
  $w = New-Object IO.StreamWriter((Join-Path $dir $file), $false, $utf8)
  $n = 0
  try {
    $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 3600
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $sql
    $r = $cmd.ExecuteReader()
    do {
      if ($r.FieldCount -gt 0) {
        if ($Header) { $w.WriteLine((@(for ($i = 0; $i -lt $r.FieldCount; $i++) { $r.GetName($i) }) -join '|')) }
        while ($r.Read()) {
          $w.WriteLine((@(for ($i = 0; $i -lt $r.FieldCount; $i++) { Clean $r.GetValue($i) }) -join '|'))
          $n++
          if ($n % 5000 -eq 0) { Write-Host ("            {0}: {1:N0} rows" -f $file, $n) -ForegroundColor DarkGray }
        }
      }
    } while ($r.NextResult())
    $r.Close()
  } finally { $w.Close(); $cn.Close() }
  return $n
}
function Query([string]$sql) {
  $cn = New-Object System.Data.SqlClient.SqlConnection ("Server=$SqlInstance;Database=sde;$(if ($SqlUser) { "User ID=$SqlUser;Password=$SqlPassword;TrustServerCertificate=True;" } else { 'Integrated Security=SSPI;' })Application Name=readonly-export;ApplicationIntent=ReadOnly;Connect Timeout=15")
  $cn.Open()
  try {
    $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 3600
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $sql
    $t = New-Object Data.DataTable; $t.Load($cmd.ExecuteReader())
    return ,$t   # the comma keeps PowerShell from unrolling the table into rows
  } finally { $cn.Close() }
}

# DEFAULT-version reconstruction (mirrors cadastre-import sde::current_rows_sql). The deletes
# table may be missing for a class that never had a delete, so it is checked first.
function CurrentRows([string]$owner, [string]$business, [int]$reg, [string]$columns) {
  $a = "$owner.a$reg"; $d = "$owner.D$reg"
  $hasD = (Query "SELECT CASE WHEN OBJECT_ID('$d') IS NULL THEN 0 ELSE 1 END AS x").Rows[0].x -eq 1
  $notDeletedBase = if ($hasD) { "WHERE NOT EXISTS (SELECT 1 FROM $d d WHERE d.SDE_DELETES_ROW_ID = b.OBJECTID AND d.SDE_STATE_ID = 0 AND d.DELETED_AT IN (SELECT lineage_id FROM #L))" } else { '' }
  $notDeletedAdd = if ($hasD) { "AND NOT EXISTS (SELECT 1 FROM $d d WHERE d.SDE_DELETES_ROW_ID = a.OBJECTID AND d.SDE_STATE_ID = a.SDE_STATE_ID AND d.DELETED_AT IN (SELECT lineage_id FROM #L))" } else { '' }
  @"
IF OBJECT_ID('tempdb..#L') IS NOT NULL DROP TABLE #L;
DECLARE @default_state bigint = (SELECT state_id FROM sde.SDE_versions WHERE name = 'DEFAULT');
SELECT lineage_id INTO #L FROM sde.SDE_state_lineages
 WHERE lineage_name = (SELECT lineage_name FROM sde.SDE_states WHERE state_id = @default_state);
CREATE CLUSTERED INDEX ix_L ON #L(lineage_id);
SELECT $columns FROM $owner.$business b $notDeletedBase
UNION ALL
SELECT $columns FROM $a a WHERE a.SDE_STATE_ID IN (SELECT lineage_id FROM #L) $notDeletedAdd;
"@
}

$t0 = Get-Date
Write-Host "Exporting from $SqlInstance (read-only) to $dir" -ForegroundColor Yellow
$reg = Query "SELECT registration_id, owner, table_name FROM sde.SDE_table_registry WHERE table_name IN ('PLATEAU_STATE_PARCELS','PARCELATTRIBUTETABLE','PLAGIS_COORDINATES','PLATEAU_STATE_POINTS')"
function RegOf([string]$t) { $r = $reg | Where { $_.table_name -eq $t } | Select -First 1; if (-not $r) { throw "$t is not registered in sde" }; $r }
$P = RegOf 'PLATEAU_STATE_PARCELS'; $A = RegOf 'PARCELATTRIBUTETABLE'; $C = RegOf 'PLAGIS_COORDINATES'; $T = RegOf 'PLATEAU_STATE_POINTS'

# Survey points (the beacons of the parcel fabric), DEFAULT version. X/Y are the stored attribute
# coordinates; SHAPE is exported too, for points whose attributes are blank.
function ExportPoints {
  $tc = "OBJECTID, CONVERT(varchar(36), PointID) AS PointID, PointName, Category, Type, Units, CONVERT(float, Weight) AS Weight, CONVERT(varchar(36), ControlPointID) AS ControlPointID, CONVERT(varchar(36), JobID) AS JobID, TempID, CONVERT(float, X) AS X, CONVERT(float, Y) AS Y, CONVERT(float, Z) AS Z, SHAPE.STX AS ShapeX, SHAPE.STY AS ShapeY"
  Export (CurrentRows $T.owner $T.table_name $T.registration_id $tc) 'points.psv' -Header
}
if ($Only -eq 'points') {
  Write-Host "[1/1] survey points / beacons (DEFAULT version) ..." -ForegroundColor Cyan
  $nt = ExportPoints
  $line = "points.psv rows: $nt (exported $(Get-Date -Format s), took $([int]((Get-Date) - $t0).TotalSeconds)s)"
  [IO.File]::WriteAllLines((Join-Path $dir 'points-summary.txt'), [string[]]@($line), $utf8)
  Write-Host "  $line"; Write-Host "`nDone. Files are in $dir" -ForegroundColor Green
  return
}

Write-Host "[1/5] parcels (DEFAULT version) ..." -ForegroundColor Cyan
$pc = "OBJECTID, UPIN, Status, CONVERT(varchar(36), JobID) AS JobID, PlanNo, CadZone, PlotNo, BlockNo, SHAPE.STSrid AS srid, SHAPE.STAsText() AS wkt"
$np = Export (CurrentRows $P.owner $P.table_name $P.registration_id $pc) 'parcels.psv'

Write-Host "[2/5] parcel attributes incl. registry links (DEFAULT version) ..." -ForegroundColor Cyan
$ac = "OBJECTID, DatasetName, RegionName, CONVERT(varchar(36), ParcelID) AS ParcelID, UPIN, ParcelName, Parent, PropertyNumber, PropertyType, Plot, Block, Location, ParcelArea, AreaUnit, PlanNumber, LGA, Status, CanRollback, IsStrata, MutationType, CreationDate, LastEditDate, ConsolidationDate, ArchiveReason, ArchiveDate, LRSID, LinkageStatus, LinkageDate, SurveyorGeneral"
$na = Export (CurrentRows $A.owner $A.table_name $A.registration_id $ac) 'parcel-attributes.psv' -Header

Write-Host "[3/5] control coordinates ..." -ForegroundColor Cyan
$nb = Export "SELECT Alias, CONVERT(float, X) AS X, CONVERT(float, Y) AS Y FROM $($C.owner).$($C.table_name);" 'beacons.psv'

Write-Host "[4/5] survey points / beacons (DEFAULT version) ..." -ForegroundColor Cyan
$nt = ExportPoints

Write-Host "[5/5] every UPIN ever issued (all versions and history states) ..." -ForegroundColor Cyan
$pa = "$($P.owner).a$($P.registration_id)"; $aa = "$($A.owner).a$($A.registration_id)"
$nu = Export @"
SELECT UPIN, MIN(src) AS first_source, COUNT(*) AS occurrences FROM (
  SELECT UPIN, 'parcels-base' AS src FROM $($P.owner).$($P.table_name)
  UNION ALL SELECT UPIN, 'parcels-history' FROM $pa
  UNION ALL SELECT UPIN, 'attributes-base' FROM $($A.owner).$($A.table_name)
  UNION ALL SELECT UPIN, 'attributes-history' FROM $aa
) u WHERE UPIN IS NOT NULL AND LTRIM(RTRIM(UPIN)) NOT IN ('', '0') GROUP BY UPIN ORDER BY UPIN;
"@ 'upins-issued.psv' -Header

Write-Host "Summary ..." -ForegroundColor Cyan
$sum = @()
$sum += "Aumentum export of $env:COMPUTERNAME ($SqlInstance) at $(Get-Date -Format s), took $([int]((Get-Date) - $t0).TotalSeconds)s"
$sum += "parcels.psv rows: $np"; $sum += "parcel-attributes.psv rows: $na"; $sum += "beacons.psv rows: $nb"; $sum += "points.psv rows: $nt"; $sum += "upins-issued.psv rows: $nu"
$sum += "DEFAULT state: " + (Query "SELECT state_id FROM sde.SDE_versions WHERE name='DEFAULT'").Rows[0].state_id
$sum += "versions: " + (Query "SELECT COUNT(*) AS n FROM sde.SDE_versions").Rows[0].n
$sum += ""; $sum += "parcels by Status (DEFAULT), counted from parcels.psv:"
$byStatus = @{}
foreach ($line in [IO.File]::ReadLines((Join-Path $dir 'parcels.psv'))) { $st = $line.Split('|')[2]; if ($byStatus.ContainsKey($st)) { $byStatus[$st]++ } else { $byStatus[$st] = 1 } }
foreach ($k in ($byStatus.Keys | Sort-Object)) { $sum += ("  Status {0}: {1}" -f $k, $byStatus[$k]) }
[IO.File]::WriteAllLines((Join-Path $dir 'export-summary.txt'), [string[]]$sum, $utf8)
$sum | Select -First 6 | % { Write-Host "  $_" }
Write-Host "`nDone. Files are in $dir" -ForegroundColor Green
