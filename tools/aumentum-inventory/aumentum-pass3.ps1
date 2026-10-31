<#
  PASS 3 - READ-ONLY. Targeted capture of:
    (a) Aumentum Desktop/Server/WebAdmin/GISViewer configuration files (secrets masked),
    (b) the Aumentum PDF guides (COPIED to -OutDir\aumentum-docs; originals untouched),
    (c) SQL Server metadata for MCServer and sde (SELECT-only, via sqlcmd, Windows auth).
  Changes nothing on this server. PowerShell 4 compatible.
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'), [string]$SqlInstance = '')
$ErrorActionPreference = 'SilentlyContinue'
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where { $_.Name -notmatch 'WID|SQLEXPRESS|FDLauncher|OLAP' -and $_.Status -eq 'Running' } | Select -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
Write-Host "Using SQL Server instance: $SqlInstance" -ForegroundColor Yellow
$out = Join-Path $OutDir ("aumentum-pass3-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format yyyyMMdd-HHmm))
"Aumentum pass 3 of $env:COMPUTERNAME started $(Get-Date); SQL instance $SqlInstance" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $out" -ForegroundColor Red; exit 1 }

$script:n = 0; $total = 11
function Section([string]$title, [scriptblock]$body) {
  $script:n++; $t0 = Get-Date
  Write-Host ("[{0:HH:mm:ss}] {1,2}/{2}  {3} ..." -f $t0, $script:n, $total, $title) -ForegroundColor Cyan
  $text = try { & $body *>&1 | Out-String -Width 400 } catch { "ERROR: $_" }
  "`r`n==================== $title ====================`r`n$text" | Out-File $out -Append -Encoding UTF8
  Write-Host ("            done in {0:N0}s" -f ((Get-Date) - $t0).TotalSeconds) -ForegroundColor DarkGray
}
function Mask($s) {
  $s -replace '(?i)((password|pwd|secret|apikey|api_key|token)\s*[=:]\s*)[^;"''<\s]+', '$1****' `
     -replace '(?i)(<(password|pwd|secret)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|secret)"\s*value\s*=\s*")[^"]*', '$1****' `
     -replace '(?i)("(password|pwd|secret)"\s*:\s*")[^"]*', '$1****'
}
function Dump([string]$path, [int]$maxKB = 300) {
  Get-ChildItem $path -Force | Where { -not $_.PSIsContainer -and $_.Length -lt ($maxKB * 1KB) } | % {
    "----- $($_.FullName) ($($_.Length) bytes, $($_.LastWriteTime)) -----"; Mask ([IO.File]::ReadAllText($_.FullName))
  }
}
# sqlcmd: PowerShell 4 has no .Source on CommandInfo, so resolve the path explicitly.
$sqlcmdPath = (Get-Command sqlcmd.exe -ErrorAction SilentlyContinue | Select -First 1).Path
if (-not $sqlcmdPath) { $sqlcmdPath = Get-ChildItem 'C:\Program Files\Microsoft SQL Server\*\Tools\Binn\SQLCMD.EXE','C:\Program Files (x86)\Microsoft SQL Server\*\Tools\Binn\SQLCMD.EXE' | Select -First 1 -Expand FullName }
function Sql([string]$db, [string]$q) {
  if (-not $sqlcmdPath) { return "sqlcmd.exe not found" }
  & $sqlcmdPath -S $SqlInstance -E -d $db -W -s "|" -l 15 -t 300 -Q ("SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $q) 2>&1
}
$AUM = 'C:\Program Files (x86)\AUMENTUM\CADASTRE'

Section "DESKTOP (ArcMap extension) CONFIG" {
  Dump "$AUM\Desktop\*.xml"; Dump "$AUM\Desktop\*.config"; Dump "$AUM\Desktop\*.Setting"
  Dump "$AUM\Desktop\LicenseManager\*.config"
}
Section "SERVER (DocSvr) CONFIG, etc, Email, Batch files, Documents\Server.txt" {
  Dump "$AUM\Server\*.config"; Dump "$AUM\Server\*.xml"; Dump "$AUM\Server\etc\*"; Dump "$AUM\Server\Email\*"
  Dump "$AUM\Server\Batch files\*.ps1"; Dump "$AUM\Server\Batch files\*.bat"; Dump "$AUM\Documents\Server.txt"
  "### Server\data (document store) - per-year counts and sizes"
  Get-ChildItem "$AUM\Server\data" -Directory | % { $f = Get-ChildItem $_.FullName -Recurse -File; [pscustomobject]@{Year=$_.Name; Files=@($f).Count; MB=[math]::Round((($f | Measure Length -Sum).Sum)/1MB)} } | Format-Table -Auto
  "### Server\data sample file types"
  Get-ChildItem "$AUM\Server\data" -Recurse -File | Group Extension | Sort Count -Descending | Select Count, Name | Format-Table -Auto
  "### root of CADASTRE folder"
  Get-ChildItem $AUM -Force | Select Name, Length, LastWriteTime | Format-Table -Auto
}
Section "WEBADMIN pages + GISViewer App_Data (structure and config)" {
  Get-ChildItem "$AUM\WebAdmin" -Force | Select Name, Length, LastWriteTime | Format-Table -Auto
  Get-ChildItem 'C:\inetpub\wwwroot\GISViewer' -Force | Select Name, Length, LastWriteTime | Format-Table -Auto
  Get-ChildItem 'C:\inetpub\wwwroot\GISViewer\App_Data' -Recurse -Force | Select FullName, Length | Format-Table -Auto -Wrap
  Get-ChildItem 'C:\inetpub\wwwroot\GISViewer\App_Data' -Recurse -File | Where { $_.Length -lt 200KB -and $_.Extension -match 'config|xml|json' } | % { "----- $($_.FullName) -----"; Mask ([IO.File]::ReadAllText($_.FullName)) }
}
Section "COPY Aumentum PDF guides to the shared folder (read-only on this server)" {
  $dest = Join-Path $OutDir 'aumentum-docs'
  New-Item -ItemType Directory -Path $dest -Force | Out-Null
  Get-ChildItem "$AUM\Documents\*.pdf" | Where { $_.Name -notmatch ' - Copy' } | % { Copy-Item $_.FullName $dest -Force; "copied $($_.Name) ($([math]::Round($_.Length/1MB,1)) MB)" }
}
Section "SQL: instance, databases, logins" {
  "sqlcmd: $sqlcmdPath"
  Sql 'master' "SELECT @@SERVERNAME AS server, SERVERPROPERTY('ProductVersion') AS ver, SERVERPROPERTY('ProductLevel') AS lvl, SERVERPROPERTY('Edition') AS edition, SERVERPROPERTY('IsIntegratedSecurityOnly') AS winauth_only;
  SELECT d.name, d.state_desc, d.recovery_model_desc, d.compatibility_level, d.create_date, CAST(SUM(f.size)*8/1024 AS int) AS size_mb FROM sys.databases d JOIN sys.master_files f ON f.database_id=d.database_id GROUP BY d.name,d.state_desc,d.recovery_model_desc,d.compatibility_level,d.create_date ORDER BY d.name;
  SELECT name, type_desc, is_disabled, create_date FROM sys.server_principals WHERE type IN ('S','U','G') AND name NOT LIKE '##%' ORDER BY name;
  SELECT database_name, type, MAX(backup_finish_date) AS last_backup FROM msdb.dbo.backupset GROUP BY database_name, type ORDER BY database_name;
  SELECT name, port, type_desc, state_desc FROM sys.tcp_endpoints;"
}
Section "SQL: MCServer tables (row counts) and programmable objects" {
  Sql 'MCServer' "SELECT s.name+'.'+t.name AS tbl, SUM(p.rows) AS rows FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name ORDER BY s.name,t.name;
  SELECT s.name+'.'+o.name AS obj, o.type_desc FROM sys.objects o JOIN sys.schemas s ON s.schema_id=o.schema_id WHERE o.type IN ('V','P','FN','TF','IF','TR') ORDER BY o.type_desc, obj;"
}
Section "SQL: MCServer columns + foreign keys" {
  Sql 'MCServer' "SELECT TABLE_SCHEMA+'.'+TABLE_NAME AS tbl, COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH AS len, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION;
  SELECT fk.name, OBJECT_NAME(fk.parent_object_id) AS child, OBJECT_NAME(fk.referenced_object_id) AS parent FROM sys.foreign_keys fk ORDER BY child;"
}
Section "SQL: MCServer small lookup/config tables (TOP 50 of tables with 1-300 rows)" {
  $list = Sql 'MCServer' "SELECT s.name+'.'+t.name FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name HAVING SUM(p.rows) BETWEEN 1 AND 300 ORDER BY 1;"
  $list | Where { $_ -match '^\w+\.\w+$' } | Select -First 100 | % {
    "### $_"; Mask ((Sql 'MCServer' "SELECT TOP 50 * FROM $_;") -join "`r`n")
  }
}
Section "SQL: MCServer activity by month (tables with a date column, last 36 months)" {
  $cands = Sql 'MCServer' "SELECT TOP 40 TABLE_SCHEMA+'.'+TABLE_NAME+'|'+COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE DATA_TYPE IN ('datetime','datetime2','date') AND (COLUMN_NAME LIKE '%creat%' OR COLUMN_NAME LIKE '%start%' OR COLUMN_NAME LIKE '%date%') ORDER BY TABLE_NAME;"
  $cands | Where { $_ -match '^\w+\.\w+\|\w+$' } | % {
    $t, $c = $_.Split('|')
    "### $t.$c"; Sql 'MCServer' "SELECT CONVERT(char(7),[$c],120) AS ym, COUNT(*) AS n FROM $t WHERE [$c] >= DATEADD(month,-36,GETDATE()) GROUP BY CONVERT(char(7),[$c],120) ORDER BY 1;"
  }
}
Section "SQL: sde geodatabase metadata" {
  Sql 'sde' "SELECT registration_id, owner, table_name, object_flags, rowid_column FROM sde.SDE_table_registry ORDER BY table_name;
  SELECT l.layer_id, l.owner, l.table_name, l.spatial_column, l.srid, r.auth_srid FROM sde.SDE_layers l LEFT JOIN sde.SDE_spatial_references r ON r.srid=l.srid ORDER BY l.table_name;
  SELECT name, owner, state_id, status, parent_name, creation_time, description FROM sde.SDE_versions ORDER BY creation_time;
  SELECT t.Name AS item_type, i.Name, i.Path FROM sde.GDB_ITEMS i JOIN sde.GDB_ITEMTYPES t ON t.UUID=i.Type ORDER BY t.Name, i.Name;"
}
Section "END" { "Finished $(Get-Date)" }
Write-Host "`nDone. Report written to: $out" -ForegroundColor Green
