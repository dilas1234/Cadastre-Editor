<#
  AUMENTUM JOB SERVICE CHECK - READ-ONLY. Run inside the Remote Desktop session on PRODUCTIONMC
  (10.10.10.6) via RUN-ME-JOBCHECK.cmd. Changes nothing.
  Why: the job service answers GetParcelAttributesByUPIN with "Object reference not set to an
  instance of an object" for every UPIN, although SQL Server, the sde database and the services are
  up (dbcheck 2026-11-10 16:38), and restarting the Aumentum service and recycling the app pools did
  not help. This finds the stack trace of that error in Aumentum's own logs and checks the versioned
  geodatabase Aumentum reads parcels through (states, lineages, delta tables, the versioned view).
  Writes <OutDir>\aumentum-capture\jobcheck-<host>-<time>.txt. Passwords are never printed.
  PowerShell 4 compatible (Windows Server 2012 R2).
#>
param([string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path), [datetime]$Since = '2026-11-05')
$ErrorActionPreference = 'SilentlyContinue'
$dir = Join-Path $OutDir 'aumentum-capture'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$out = Join-Path $dir ("jobcheck-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format 'yyyyMMdd-HHmm'))
"Aumentum job service check on $env:COMPUTERNAME at $(Get-Date)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $dir" -ForegroundColor Red; exit 1 }
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
function Hdr([string]$s) { Log ''; Log "===== $s =====" 'Cyan' }
function Hide([string]$s) { $s -replace '(?i)(password|pwd|encrypted_password)\s*=\s*[^;"<]*', '$1=***' }
$roots = @("${env:ProgramFiles(x86)}\AUMENTUM", "$env:ProgramFiles\AUMENTUM", "$env:SystemDrive\inetpub", 'D:\', 'E:\') | Where-Object { Test-Path $_ }

# 1. Where the job service lives: IIS sites and applications, and the .asmx/.svc files.
Hdr '1. IIS sites and applications'
$appcmd = "$env:windir\System32\inetsrv\appcmd.exe"
if (Test-Path $appcmd) {
  & $appcmd list site | ForEach-Object { Log $_ }
  & $appcmd list app | ForEach-Object { Log $_ }
  & $appcmd list vdir | ForEach-Object { Log $_ }
}
Hdr '1b. Job service files (*Job*.asmx / *.svc)'
$svc = foreach ($r in $roots) { Get-ChildItem -Path $r -Recurse -Include '*Job*.asmx', '*Job*.svc' -ErrorAction SilentlyContinue | Select-Object -First 20 }
$svc | ForEach-Object { Log ("{0}  ({1})" -f $_.FullName, $_.LastWriteTime) 'White' }

# 2. The job service's config: database and log settings (passwords hidden).
Hdr '2. Job service web.config (connections, log files, Aumentum keys)'
$configs = $svc | ForEach-Object { Join-Path $_.DirectoryName 'web.config' } | Sort-Object -Unique | Where-Object { Test-Path $_ }
$logFiles = @()
foreach ($c in $configs) {
  Log $c 'White'
  $n = 0
  Get-Content $c | ForEach-Object {
    $n++
    if ($_ -match '(?i)connectionString|Data Source|<add key=|file value=|<file |logfile|log4net|sde|server=|instance') { Log ("   {0}: {1}" -f $n, (Hide $_.Trim())) }
    if ($_ -match '(?i)<file\s+value="([^"]+)"|file\s*=\s*"([^"]+\.(log|txt))"') { $logFiles += ($matches[1], $matches[2] | Where-Object { $_ }) }
  }
}

# 3. Aumentum's own logs: the stack trace of "Object reference not set".
Hdr "3. Aumentum log files written since $($Since.ToString('yyyy-MM-dd'))"
$cands = @()
foreach ($l in $logFiles) {
  $p = [Environment]::ExpandEnvironmentVariables($l)
  if (Test-Path $p) { $cands += Get-Item $p } elseif (Test-Path (Split-Path $p)) { $cands += Get-ChildItem (Split-Path $p) -File }
}
foreach ($r in $roots) { $cands += Get-ChildItem -Path $r -Recurse -Include '*.log', '*.txt' -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match '(?i)log' -and $_.FullName -notmatch '(?i)\\W3SVC|\\HTTPERR|arcgisserver|aumentum-capture' } }
$cands = $cands | Where-Object { $_ -and $_.LastWriteTime -ge $Since -and $_.Length -lt 500MB } | Sort-Object FullName -Unique | Sort-Object LastWriteTime -Descending | Select-Object -First 40
$cands | ForEach-Object { Log ("{0}  ({1}, {2:N0} KB)" -f $_.FullName, $_.LastWriteTime, ($_.Length / 1KB)) }

Hdr '3b. "Object reference" / exceptions in those logs (newest 6, with the stack trace)'
$shown = 0
foreach ($f in $cands) {
  if ($shown -ge 6) { break }
  $hits = Select-String -Path $f.FullName -Pattern 'Object reference not set|NullReferenceException|GetParcelAttributesByUPIN' -Context 3, 25 | Select-Object -Last 3
  foreach ($h in $hits) {
    if ($shown -ge 6) { break }
    $shown++
    Log ("--- {0} line {1}" -f $f.FullName, $h.LineNumber) 'White'
    $h.Context.PreContext | ForEach-Object { Log ('   ' + (Hide $_)) }
    Log ('>  ' + (Hide $h.Line)) 'Yellow'
    $h.Context.PostContext | ForEach-Object { Log ('   ' + (Hide $_)) }
  }
}
if ($shown -eq 0) { Log 'No "Object reference" found in the log files above.' 'Yellow' }

Hdr '3c. Application event log: .NET / ASP.NET errors with "Object reference" (full text, newest 4)'
Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $Since } -ErrorAction SilentlyContinue |
  Where-Object { $_.Message -match 'Object reference|NullReference' } | Select-Object -First 4 | ForEach-Object {
    Log ("--- {0}  {1} id={2}" -f $_.TimeCreated, $_.ProviderName, $_.Id) 'White'
    $m = Hide $_.Message; if ($m.Length -gt 4000) { $m = $m.Substring(0, 4000) + ' ...' }
    $m -split "`n" | ForEach-Object { Log ('   ' + $_.TrimEnd()) }
  }

# 4. The versioned geodatabase Aumentum reads parcels through.
function Ask([string]$label, [string]$sql, [int]$timeout = 60) {
  Log "-- $label" 'White'
  $t0 = Get-Date
  try {
    $cn = New-Object System.Data.SqlClient.SqlConnection("Server=PRODUCTIONMC\PRODUCTIONMC;Database=sde;Integrated Security=True;Connect Timeout=10;Application Name=aumentum-jobcheck")
    $cn.Open()
    $cmd = $cn.CreateCommand(); $cmd.CommandText = "SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; $sql"; $cmd.CommandTimeout = $timeout
    $rd = $cmd.ExecuteReader(); $n = 0
    do { while ($rd.Read() -and $n -lt 40) { $n++; $vals = for ($i = 0; $i -lt $rd.FieldCount; $i++) { $rd.GetValue($i) }; Log ('   ' + ($vals -join ' | ')) 'Green' } } while ($rd.NextResult())
    $cn.Close()
    Log ("   ({0:N1} s)" -f ((Get-Date) - $t0).TotalSeconds)
  } catch { Log ("   FAILED after {0:N1} s: {1}" -f ((Get-Date) - $t0).TotalSeconds, $_.Exception.Message.Split("`n")[0]) 'Red' }
}
Hdr '4. Versioned geodatabase (sde)'
Ask 'states and lineages (grow until the geodatabase is compressed)' "SELECT 'states', COUNT_BIG(*) FROM sde.SDE_states UNION ALL SELECT 'state_lineages', COUNT_BIG(*) FROM sde.SDE_state_lineages UNION ALL SELECT 'versions', COUNT_BIG(*) FROM sde.SDE_versions UNION ALL SELECT 'state_locks', COUNT_BIG(*) FROM sde.SDE_state_locks UNION ALL SELECT 'process_info', COUNT_BIG(*) FROM sde.SDE_process_information;"
Ask 'versions (newest 25)' "SELECT TOP 25 owner + '.' + name, state_id, status, parent_owner + '.' + parent_name, creation_time FROM sde.SDE_versions ORDER BY creation_time DESC;"
Ask 'DEFAULT version' "SELECT owner + '.' + name, state_id, creation_time FROM sde.SDE_versions WHERE name = 'DEFAULT';"
Ask 'last compress' "SELECT TOP 5 compress_start, compress_end, start_state_count, end_state_count, compress_status FROM sde.SDE_compress_log ORDER BY compress_start DESC;"
Ask 'parcel layers: registration and delta tables (adds / deletes)' @"
DECLARE @s nvarchar(max) = N'';
SELECT @s = @s + N'SELECT ''' + table_name + N''' AS tbl, ' + CAST(registration_id AS nvarchar(20)) + N' AS reg, ''' + ISNULL(imv_view_name, N'') + N''' AS view_name, '
  + N'(SELECT COUNT_BIG(*) FROM sde.' + table_name + N') AS base_rows, '
  + N'(SELECT COUNT_BIG(*) FROM sde.a' + CAST(registration_id AS nvarchar(20)) + N') AS adds, '
  + N'(SELECT COUNT_BIG(*) FROM sde.D' + CAST(registration_id AS nvarchar(20)) + N') AS deletes UNION ALL '
FROM sde.SDE_table_registry WHERE table_name LIKE 'PLATEAU_STATE_PARCELS%' OR table_name LIKE 'PLATEAU_STATE_JOBS%';
IF LEN(@s) > 0 BEGIN SET @s = LEFT(@s, LEN(@s) - 10); EXEC (@s); END
"@ 120
Ask 'parcels as Aumentum reads them (versioned view, DEFAULT), with time' @"
DECLARE @v sysname = (SELECT imv_view_name FROM sde.SDE_table_registry WHERE table_name = 'PLATEAU_STATE_PARCELS');
IF @v IS NOT NULL EXEC (N'SELECT ''view rows'', COUNT_BIG(*) FROM sde.' + @v);
"@ 120
Ask 'one UPIN through the versioned view (PL000000023899, which failed at 15:52)' @"
DECLARE @v sysname = (SELECT imv_view_name FROM sde.SDE_table_registry WHERE table_name = 'PLATEAU_STATE_PARCELS');
IF @v IS NOT NULL EXEC (N'SELECT TOP 5 OBJECTID, ParcelID, UPIN, Status, JobID, LastEditDate FROM sde.' + @v + N' WHERE UPIN = ''PL000000023899''');
"@ 120
Ask 'largest tables in sde (rows)' "SELECT TOP 15 s.name + '.' + t.name, SUM(p.rows) FROM sys.tables t JOIN sys.schemas s ON s.schema_id = t.schema_id JOIN sys.partitions p ON p.object_id = t.object_id AND p.index_id IN (0,1) GROUP BY s.name, t.name ORDER BY SUM(p.rows) DESC;"

Log ''
Log "Done. Report: $out" 'Green'
