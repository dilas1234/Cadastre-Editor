<#
  PASS 2 - READ-ONLY deep inventory of the Aumentum Cadastre install, ArcGIS Server site and
  the local SQL Server databases (sde, MCServer).
  - File system / registry: Get-* only. SQL: SELECT-only queries via sqlcmd (Windows auth).
  - Changes nothing on the server. Only write: the report file in -OutDir.
  - Secrets in config/log text are masked. PowerShell 4 compatible.
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'), [string]$SqlInstance = '')
# Auto-detect the main SQL Server instance (skips Windows Internal Database and SQLEXPRESS).
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where { $_.Name -notmatch 'WID|SQLEXPRESS|MSSQLFDLauncher|MSSQLServerOLAP' -and $_.Status -eq 'Running' } | Select -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
Write-Host "Using SQL Server instance: $SqlInstance" -ForegroundColor Yellow
$ErrorActionPreference = 'SilentlyContinue'
$out = Join-Path $OutDir ("aumentum-deep-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format yyyyMMdd-HHmm))
"Aumentum deep inventory of $env:COMPUTERNAME started $(Get-Date)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $out" -ForegroundColor Red; exit 1 }

$script:n = 0; $total = 16
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
# Depth-limited recursive listing (depth 0 = direct children).
function Walk([string]$path, [int]$max = 3, [int]$depth = 0) {
  foreach ($i in (Get-ChildItem -LiteralPath $path -Force)) {
    $i
    if ($i.PSIsContainer -and ($depth + 1) -lt $max) { Walk $i.FullName $max ($depth + 1) }
  }
}
function Rel($item, $root) { $item.FullName.Substring($root.Length) }
function Sql([string]$db, [string]$q) {
  $sqlcmd = Get-Command sqlcmd.exe
  if (-not $sqlcmd) { return "sqlcmd.exe not found" }
  & $sqlcmd.Path -S $SqlInstance -E -d $db -W -s "|" -l 15 -t 120 -Q ("SET NOCOUNT ON; " + $q) 2>&1
}

$AUM  = 'C:\Program Files (x86)\AUMENTUM\CADASTRE'
$code = '^\.(exe|dll)$'
$cfg  = '^\.(config|xml|ini|json|settings|udl|sde|properties|xsd|xslt?|sql|rpt|rdlc|mxd|txt)$'

Section "AUMENTUM FOLDER TREE (4 levels)" {
  Walk $AUM 4 | Select -First 3000 | Select @{n='Path';e={Rel $_ $AUM}}, Length, LastWriteTime | Format-Table -Auto -Wrap
}
Section "AUMENTUM ASSEMBLIES: version / product / .NET runtime" {
  Walk $AUM 5 | Where { $_.Extension -match $code } | Select -First 800 | % {
    $an = try { [Reflection.AssemblyName]::GetAssemblyName($_.FullName) } catch { $null }
    [pscustomobject]@{ File = Rel $_ $AUM; FileVer = $_.VersionInfo.FileVersion; Product = $_.VersionInfo.ProductName
      Company = $_.VersionInfo.CompanyName; Managed = [bool]$an; Arch = if ($an) { $an.ProcessorArchitecture } else { '' } }
  } | Format-Table -Auto -Wrap
}
Section "AUMENTUM ASSEMBLY REFERENCES (what each .NET dll depends on)" {
  Walk $AUM 5 | Where { $_.Extension -match $code } | Select -First 400 | % {
    try {
      $a = [Reflection.Assembly]::ReflectionOnlyLoadFrom($_.FullName)
      "## $(Rel $_ $AUM)  [$($a.ImageRuntimeVersion)]"
      ($a.GetReferencedAssemblies() | % { "    $($_.Name) $($_.Version)" }) -join "`r`n"
    } catch { }
  }
}
Section "AUMENTUM CONFIG / TEXT FILES (secrets masked)" {
  Walk $AUM 5 | Where { -not $_.PSIsContainer -and $_.Extension -match $cfg -and $_.Extension -ne '.mxd' -and $_.Length -lt 300KB } | Select -First 150 |
    % { "----- $($_.FullName) ($($_.Length) bytes, $($_.LastWriteTime)) -----"; Mask ([IO.File]::ReadAllText($_.FullName)) }
}
Section "AUMENTUM LOGS (newest 15 files, last 80 lines each, masked)" {
  $logs = @(Walk $AUM 5) + @(Walk 'C:\ProgramData' 2 | Where { $_.Name -match 'Aumentum|Cadastre|ILS|MultiCadastre' } | % { Walk $_.FullName 4 })
  $logs | Where { -not $_.PSIsContainer -and $_.Extension -match '^\.(log|txt)$' } | Sort LastWriteTime -Descending | Select -First 15 |
    % { "----- $($_.FullName) ($($_.LastWriteTime)) -----"; Mask ((Get-Content $_.FullName -Tail 80) -join "`r`n") }
}
Section "AUMENTUM USER-PROFILE DATA (AppData folders)" {
  Get-ChildItem 'C:\Users' -Directory | % {
    foreach ($sub in 'AppData\Roaming','AppData\Local') {
      $p = Join-Path $_.FullName $sub
      Get-ChildItem $p -Directory | Where { $_.Name -match 'Aumentum|Cadastre|ILS|Thomson|MultiCadastre|ESRI' } |
        % { "### $($_.FullName)"; Walk $_.FullName 3 | Select -First 200 | Select @{n='Path';e={Rel $_ $p}}, Length, LastWriteTime | Format-Table -Auto -Wrap | Out-String }
    }
  }
}
Section "ARCMAP EXTENSION REGISTRATION (Aumentum COM classes)" {
  Get-ChildItem 'Registry::HKEY_CLASSES_ROOT\Wow6432Node\CLSID' | % {
    $ip = Get-ItemProperty (Join-Path $_.PSPath 'InprocServer32')
    if ($ip.CodeBase -match 'AUMENTUM' -or $ip.Assembly -match 'ILS|MultiCadastre|Aumentum' -or $ip.'(default)' -match 'AUMENTUM') {
      [pscustomobject]@{ CLSID = $_.PSChildName; Name = (Get-ItemProperty $_.PSPath).'(default)'; Class = $ip.Class; Assembly = $ip.Assembly }
    }
  } | Format-Table -Auto -Wrap
}
Section "IIS: WebAdmin + GISViewer web.config (masked) and app pool identities" {
  Import-Module WebAdministration
  Get-ChildItem IIS:\AppPools | Select Name, @{n='Identity';e={$_.processModel.identityType}}, @{n='User';e={$_.processModel.userName}} | Format-Table -Auto
  foreach ($p in "$AUM\WebAdmin", 'C:\inetpub\wwwroot\GISViewer') {
    "### $p"; Walk $p 3 | Select -First 300 | Select @{n='Path';e={Rel $_ $p}}, Length, LastWriteTime | Format-Table -Auto -Wrap | Out-String
    Walk $p 4 | Where { $_.Name -match '\.config$|\.json$|config\.xml$' -and $_.Length -lt 300KB } | Select -First 20 |
      % { "----- $($_.FullName) -----"; Mask ([IO.File]::ReadAllText($_.FullName)) }
  }
}
Section "ARCGIS SERVER SITE (config-store services, masked)" {
  $cfgXml = 'D:\Set_upConfigfile\Arcgis_server.xml'
  if (Test-Path $cfgXml) { "----- $cfgXml -----"; Mask ([IO.File]::ReadAllText($cfgXml)) }
  $stores = @('C:\arcgisserver\config-store','D:\arcgisserver\config-store','E:\arcgisserver\config-store') | Where { Test-Path $_ }
  foreach ($s in $stores) {
    "### $s"; Walk $s 2 | Select -First 300 | Select @{n='Path';e={Rel $_ $s}}, LastWriteTime | Format-Table -Auto -Wrap | Out-String
    Walk "$s\services" 3 | Where { $_.Extension -eq '.json' -and $_.Length -lt 200KB } | Select -First 60 |
      % { "----- $($_.FullName) -----"; Mask ([IO.File]::ReadAllText($_.FullName)) }
  }
}
Section "MAP DOCUMENTS (.mxd) and connection files (.sde) anywhere on C:/D:/E: top 3 levels" {
  & { foreach ($r in 'C:\Users','C:\Program Files (x86)\AUMENTUM','D:\','E:\MapServices','E:\MAPPING') {
    $depth = if ($r -eq 'D:\') { 2 } else { 3 }
    Walk $r $depth | Where { $_.Extension -match '^\.(mxd|sde|gdb|lyr)$' } | Select -First 100 | Select FullName, Length, LastWriteTime
  } } | Format-Table -Auto -Wrap
}
Section "SQL: instance, databases, sizes" {
  Sql 'master' "SELECT @@SERVERNAME AS server, SERVERPROPERTY('ProductVersion') AS ver, SERVERPROPERTY('ProductLevel') AS lvl, SERVERPROPERTY('Edition') AS edition, SERVERPROPERTY('IsIntegratedSecurityOnly') AS winauth_only;
  SELECT d.name, d.state_desc, d.recovery_model_desc, d.compatibility_level, d.create_date, CAST(SUM(f.size)*8/1024 AS int) AS size_mb
  FROM sys.databases d JOIN sys.master_files f ON f.database_id=d.database_id GROUP BY d.name,d.state_desc,d.recovery_model_desc,d.compatibility_level,d.create_date ORDER BY d.name;
  SELECT name, type_desc, is_disabled, create_date FROM sys.server_principals WHERE type IN ('S','U','G') AND name NOT LIKE '##%' ORDER BY name;
  SELECT name, port, type_desc, state_desc FROM sys.tcp_endpoints;"
}
Section "SQL: MCServer tables with row counts" {
  Sql 'MCServer' "SELECT s.name+'.'+t.name AS tbl, SUM(p.rows) AS rows FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name ORDER BY s.name,t.name;
  SELECT s.name+'.'+o.name AS obj, o.type_desc FROM sys.objects o JOIN sys.schemas s ON s.schema_id=o.schema_id WHERE o.type IN ('V','P','FN','TF','IF','TR') ORDER BY o.type_desc, obj;"
}
Section "SQL: MCServer columns + foreign keys" {
  Sql 'MCServer' "SELECT TABLE_SCHEMA+'.'+TABLE_NAME AS tbl, COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH AS len, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION;
  SELECT fk.name, OBJECT_NAME(fk.parent_object_id) AS child, OBJECT_NAME(fk.referenced_object_id) AS parent FROM sys.foreign_keys fk ORDER BY child;"
}
Section "SQL: MCServer small lookup/config tables (first 50 rows of tables with <=200 rows)" {
  $list = Sql 'MCServer' "SELECT s.name+'.'+t.name FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name HAVING SUM(p.rows) BETWEEN 1 AND 200 ORDER BY 1;"
  $list | Select -Skip 2 | Where { $_ -match '^\w+\.\w+$' } | Select -First 80 | % {
    "### $_"; Mask ((Sql 'MCServer' "SELECT TOP 50 * FROM $_;") -join "`r`n")
  }
}
Section "SQL: sde geodatabase (registry, layers, versions, items)" {
  Sql 'sde' "SELECT registration_id, owner, table_name, object_flags, rowid_column FROM sde.SDE_table_registry ORDER BY table_name;
  SELECT l.layer_id, l.owner, l.table_name, l.spatial_column, l.srid, r.auth_srid FROM sde.SDE_layers l LEFT JOIN sde.SDE_spatial_references r ON r.srid=l.srid ORDER BY l.table_name;
  SELECT name, owner, state_id, status, parent_name, creation_time, description FROM sde.SDE_versions ORDER BY creation_time;
  SELECT t.Name AS item_type, i.Name, i.Path FROM sde.GDB_ITEMS i JOIN sde.GDB_ITEMTYPES t ON t.UUID=i.Type ORDER BY t.Name, i.Name;"
}
Section "END" { "Finished $(Get-Date)" }
Write-Host "`nDone. Report written to: $out" -ForegroundColor Green
