<#
  PASS 4 - READ-ONLY. Captures what a full replica of the Aumentum Cadastre needs and pass 3 does not:
    (a) COPIES of the map templates, template images, a sample of recent per-parcel plans (.mxd)
        and any plan PDFs/JPGs, plus the ArcGIS Server Parcels.mxd  -> <OutDir>\aumentum-capture\
    (b) the full Aumentum registry tree (toolbar customisation, region/data-source settings), masked
    (c) sde: every business table's columns, geodatabase definitions (domains, subtypes,
        relationship classes, topology), ParcelAttributeTable (schema, sample, link-field values),
        the highest UPIN ever issued (base + delta tables, all statuses), job versions
    (d) MCServer: dictionary + dsssettings (masked), mid-size tables (301-5000 rows, TOP 100),
        every column that looks like a registry/transaction/UPIN link, with samples
  Only SELECT statements (READ UNCOMMITTED, so it never blocks editors). Writes only to -OutDir.
  Personal data: samples are limited to configuration/lookup tables and link columns; owner or
  payment details are not selected. PowerShell 4 compatible (Windows Server 2012 R2).
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'), [string]$SqlInstance = '', [int]$From = 1)
$ErrorActionPreference = 'SilentlyContinue'
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where { $_.Name -notmatch 'WID|SQLEXPRESS|FDLauncher|OLAP' -and $_.Status -eq 'Running' } | Select -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
Write-Host "Using SQL Server instance: $SqlInstance" -ForegroundColor Yellow
$out = Join-Path $OutDir ("aumentum-pass4-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format yyyyMMdd-HHmm))
"Aumentum pass 4 of $env:COMPUTERNAME started $(Get-Date); SQL instance $SqlInstance" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $out" -ForegroundColor Red; exit 1 }
$cap = Join-Path $OutDir 'aumentum-capture'
New-Item -ItemType Directory -Path $cap -Force | Out-Null

$script:n = 0; $total = 12
function Section([string]$title, [scriptblock]$body) {
  $script:n++; $t0 = Get-Date
  if ($script:n -lt $From) { Write-Host ("            {0,2}/{1}  {2}: skipped (-From {3})" -f $script:n, $total, $title, $From) -ForegroundColor DarkGray; return }
  Write-Host ("[{0:HH:mm:ss}] {1,2}/{2}  {3} ..." -f $t0, $script:n, $total, $title) -ForegroundColor Cyan
  $text = try { & $body *>&1 | Out-String -Width 400 } catch { "ERROR: $_" }
  "`r`n==================== $title ====================`r`n$text" | Out-File $out -Append -Encoding UTF8
  Write-Host ("            done in {0:N0}s" -f ((Get-Date) - $t0).TotalSeconds) -ForegroundColor DarkGray
}
function Mask($s) {
  $s -replace '(?i)((password|pwd|secret|apikey|api_key|token|ticket)\s*[=:]\s*)[^;"''<\s]+', '$1****' `
     -replace '(?i)(<(password|pwd|secret|ticket)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|secret|ticket)"\s*value\s*=\s*")[^"]*', '$1****'
}
function CopyTo([string]$sub, $files) {
  $dest = Join-Path $cap $sub; New-Item -ItemType Directory -Path $dest -Force | Out-Null
  foreach ($f in @($files)) {
    if (-not $f) { continue }
    # Prefix with the parent folder so same-named files from different places don't overwrite each other.
    $name = ((Split-Path (Split-Path $f.FullName -Parent) -Leaf) + '__' + $f.Name)
    Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $dest $name) -Force
    "copied {0} ({1:N0} KB, {2})" -f $f.FullName, ($f.Length / 1KB), $f.LastWriteTime
  }
}
$sqlcmdPath = (Get-Command sqlcmd.exe -ErrorAction SilentlyContinue | Select -First 1).Path
if (-not $sqlcmdPath) { $sqlcmdPath = Get-ChildItem 'C:\Program Files\Microsoft SQL Server\*\Tools\Binn\SQLCMD.EXE','C:\Program Files (x86)\Microsoft SQL Server\*\Tools\Binn\SQLCMD.EXE' | Select -First 1 -Expand FullName }
function Sql([string]$db, [string]$q, [int]$width = -1) {
  # Read-only query through .NET SqlClient with the current Windows sign-in (works the same in a
  # local or a remote-management session; errors are printed instead of silently lost).
  # Output: a header line, then one line per row, '|'-separated. $width is accepted and ignored.
  try {
    $cn = New-Object System.Data.SqlClient.SqlConnection ("Server=$SqlInstance;Database=$db;Integrated Security=SSPI;Application Name=readonly-inventory;Connect Timeout=15")
    $cn.Open()
    try {
      $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 600
      $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $q
      $r = $cmd.ExecuteReader()
      do {
        if ($r.FieldCount -gt 0) {
          (@(for ($i = 0; $i -lt $r.FieldCount; $i++) { $r.GetName($i) }) -join '|')
          while ($r.Read()) {
            (@(for ($i = 0; $i -lt $r.FieldCount; $i++) {
              if ($r.IsDBNull($i)) { 'NULL' } else { try { ([string]$r.GetValue($i)) -replace "[\r\n]+", ' ' } catch { '<unreadable>' } }
            }) -join '|')
          }
          ''
        }
      } while ($r.NextResult())
      $r.Close()
    } finally { $cn.Close() }
  } catch { "SQL ERROR ($db): $($_.Exception.Message)" }
}
$AUM = 'C:\Program Files (x86)\AUMENTUM\CADASTRE'

Section "COPY map templates, template images and template config" {
  CopyTo 'templates' (Get-ChildItem "$AUM\Desktop\Templates" -Recurse -File)
  CopyTo 'templates' (Get-ChildItem "$AUM\*.mxd", "$AUM\reserve\*.mxd" -File)
  CopyTo 'config'    (Get-ChildItem "$AUM\Desktop\*.xml" -File | Where { $_.Name -notmatch '^(log4net|System\.)' })
}
Section "COPY a sample of real plans (newest 15 PL/PSG/BP .mxd) and any plan PDF/JPG outputs" {
  # PowerShell 4 has no -Depth, and D: holds terabytes of rasters: search the top 4 levels only,
  # by wildcard paths, instead of a full recursive walk.
  function Shallow([string[]]$roots, [string[]]$names) {
    foreach ($r in $roots) { foreach ($lvl in '', '*\', '*\*\', '*\*\*\') { foreach ($nm in $names) { Get-ChildItem -Path (Join-Path $r ($lvl + $nm)) -File } } }
  }
  $roots = 'C:\Users','D:\','E:\','C:\Program Files (x86)\AUMENTUM\CADASTRE'
  $mxd = Shallow $roots @('PL*.mxd','PSG*.mxd','BP*.mxd')
  CopyTo 'plans-mxd' ($mxd | Sort LastWriteTime -Descending | Select -First 15)
  $outp = Shallow @('C:\Users','D:\','E:\') @('PL*.pdf','PSG*.pdf','BP*.pdf','PL*.jpg') | Where { $_.Length -lt 20MB }
  "### plan outputs found: $(@($outp).Count)"
  CopyTo 'plans-output' ($outp | Sort LastWriteTime -Descending | Select -First 15)
  CopyTo 'mapservices' (Get-ChildItem 'E:\MapServices\*.mxd' -File)
}
Section "Aumentum registry tree (full depth, masked)" {
  foreach ($k in 'HKLM:\SOFTWARE\Wow6432Node\Aumentum','HKLM:\SOFTWARE\Aumentum','HKLM:\SOFTWARE\Wow6432Node\Thomson Reuters','HKCU:\Software\Aumentum') {
    Get-ChildItem $k -Recurse | % {
      "[$($_.Name)]"
      $key = $_; $key.GetValueNames() | % { "  {0} = {1}" -f $_, (Mask ([string]$key.GetValue($_))) }
    }
  }
}
Section "sde: business tables and their columns" {
  Sql 'sde' "SELECT r.registration_id, r.owner, r.table_name, r.object_flags FROM sde.SDE_table_registry r WHERE r.owner <> 'sde' ORDER BY r.table_name;
  SELECT c.TABLE_SCHEMA+'.'+c.TABLE_NAME AS tbl, c.COLUMN_NAME, c.DATA_TYPE, c.CHARACTER_MAXIMUM_LENGTH AS len, c.IS_NULLABLE
  FROM INFORMATION_SCHEMA.COLUMNS c WHERE c.TABLE_SCHEMA <> 'sde' OR c.TABLE_NAME IN (SELECT table_name FROM sde.SDE_table_registry WHERE owner='sde')
  ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION;"
}
Section "sde: geodatabase definitions (domains, subtypes, relationship classes, topology, feature classes)" {
  Sql 'sde' "SELECT t.Name AS item_type, i.Name, CAST(i.Definition AS nvarchar(max)) AS definition
  FROM sde.GDB_ITEMS i JOIN sde.GDB_ITEMTYPES t ON t.UUID = i.Type
  WHERE t.Name IN ('Coded Value Domain','Range Domain','Relationship Class','Topology','Feature Dataset','Feature Class','Table')
  ORDER BY t.Name, i.Name;" 0
}
Section "sde: ParcelAttributeTable (registration, columns, row counts, sample, link-field values)" {
  Sql 'sde' "SELECT registration_id, owner, table_name FROM sde.SDE_table_registry WHERE table_name LIKE '%ATTRIBUTE%' OR table_name LIKE '%LINK%' OR table_name LIKE '%LRS%';"
  $tbls = Sql 'sde' "SELECT TABLE_SCHEMA+'.'+TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME LIKE '%ParcelAttribute%' OR TABLE_NAME LIKE '%LRS%' OR TABLE_NAME LIKE '%Link%';"
  foreach ($tbl in ($tbls | Where { $_ -match '^\w+\.\w+$' })) {
    "### $tbl"
    Sql 'sde' "SELECT COUNT(*) AS rows FROM $tbl;"
    Sql 'sde' "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA+'.'+TABLE_NAME = '$tbl' ORDER BY ORDINAL_POSITION;"
    Sql 'sde' "SELECT TOP 30 * FROM $tbl ORDER BY 1 DESC;"
    # Distinct values of status/link/LRS-like columns (these drive the linkage rules).
    $cols = Sql 'sde' "SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA+'.'+TABLE_NAME = '$tbl' AND (COLUMN_NAME LIKE '%status%' OR COLUMN_NAME LIKE '%link%' OR COLUMN_NAME LIKE '%lrs%' OR COLUMN_NAME LIKE '%pin%' OR COLUMN_NAME LIKE '%trans%' OR COLUMN_NAME LIKE '%type%');"
    foreach ($c in ($cols | Where { $_ -match '^\w+$' -and $_ -ne 'COLUMN_NAME' })) {
      "## distinct $c"; Sql 'sde' "SELECT TOP 50 [$c] AS value, COUNT(*) AS n FROM $tbl GROUP BY [$c] ORDER BY n DESC;"
    }
  }
}
Section "sde: highest UPIN ever issued (base + adds tables, every status) and UPIN format check" {
  $regs = Sql 'sde' "SELECT CAST(registration_id AS varchar)+'|'+owner+'.'+table_name FROM sde.SDE_table_registry r WHERE EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS c WHERE c.TABLE_SCHEMA=r.owner AND c.TABLE_NAME=r.table_name AND c.COLUMN_NAME='UPIN');"
  $regs | Where { $_ -match '^\d+\|' } | % {
    $id, $t = $_.Split('|'); $owner = $t.Split('.')[0]
    "### $t (registration $id)"
    Sql 'sde' "SELECT 'base' AS src, COUNT(*) AS rows, MIN(UPIN) AS min_upin, MAX(UPIN) AS max_upin, SUM(CASE WHEN UPIN LIKE 'PL[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]' THEN 0 ELSE 1 END) AS non_standard FROM $t
    UNION ALL SELECT 'adds a$id', COUNT(*), MIN(UPIN), MAX(UPIN), SUM(CASE WHEN UPIN LIKE 'PL[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]' THEN 0 ELSE 1 END) FROM $owner.a$id;"
    Sql 'sde' "SELECT TOP 20 UPIN, COUNT(*) AS n FROM (SELECT UPIN FROM $t UNION ALL SELECT UPIN FROM $owner.a$id) u WHERE UPIN NOT LIKE 'PL[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]' GROUP BY UPIN ORDER BY n DESC;"
  }
}
Section "sde: versions (open job versions = in-flight work) and per-job layers" {
  Sql 'sde' "SELECT name, owner, state_id, status, parent_name, creation_time, description FROM sde.SDE_versions ORDER BY creation_time DESC;"
  Sql 'sde' "SELECT r.registration_id, r.owner+'.'+r.table_name AS tbl FROM sde.SDE_table_registry r WHERE r.table_name LIKE '%JOB%' ORDER BY r.table_name;"
}
Section "MCServer: dictionary and dsssettings (all rows, masked)" {
  foreach ($t in 'dictionary','dsssettings') {
    "### $t"; Mask ((Sql 'MCServer' "SELECT * FROM [$t];") -join "`r`n")
  }
}
Section "MCServer: mid-size tables (301-5000 rows, TOP 100 each, masked)" {
  $list = Sql 'MCServer' "SELECT s.name+'.'+t.name FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name HAVING SUM(p.rows) BETWEEN 301 AND 5000 ORDER BY 1;"
  $list | Where { $_ -match '^\w+\.\w+$' } | Select -First 80 | % { "### $_"; Mask ((Sql 'MCServer' "SELECT TOP 100 * FROM $_;") -join "`r`n") }
}
Section "MCServer: link columns (transaction / UPIN / LRS / PIN / job instance) with samples" {
  $cols = Sql 'MCServer' "SELECT TABLE_SCHEMA+'.'+TABLE_NAME+'|'+COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE COLUMN_NAME LIKE '%trans%' OR COLUMN_NAME LIKE '%upin%' OR COLUMN_NAME LIKE '%lrs%' OR COLUMN_NAME LIKE '%pin' OR COLUMN_NAME LIKE '%registry%' OR COLUMN_NAME LIKE '%jobinstance%' ORDER BY 1;"
  $cols | Where { $_ -match '^\w+\.\w+\|\w+$' } | % {
    $t, $c = $_.Split('|'); "### $t.$c"
    Sql 'MCServer' "SELECT COUNT(*) AS rows, COUNT(DISTINCT [$c]) AS distinct_values, MIN([$c]) AS min_v, MAX([$c]) AS max_v FROM $t;"
    Sql 'MCServer' "SELECT TOP 10 [$c] FROM $t WHERE [$c] IS NOT NULL ORDER BY 1 DESC;"
  }
}
Section "END" { "Finished $(Get-Date). Copies are in $cap" }
Write-Host "`nDone. Report: $out`nCopies:  $cap" -ForegroundColor Green
