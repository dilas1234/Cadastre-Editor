<#
  AUMENTUM DATABASE CHECK - READ-ONLY. Run inside the Remote Desktop session on PRODUCTIONMC
  (10.10.10.6) via RUN-ME-DBCHECK.cmd. Changes nothing: it only reads services, connection files,
  configs, event logs and log files, and asks the databases one harmless query each.
  Why: since Nov 5-7 every parcel query fails (Aumentum job service: "Object reference not set",
  ArcGIS Aumentum/Parcels: "Error performing query operation" after ~20 s). This finds which
  database Aumentum uses (10.10.10.3 or this server's own PRODUCTIONMC instance), whether it
  answers, and what the logs say since Nov 5.
  Writes <OutDir>\aumentum-capture\dbcheck-<host>-<time>.txt (back on the Linux machine).
  Passwords are never printed. PowerShell 4 compatible (Windows Server 2012 R2).
#>
param([string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path), [datetime]$Since = '2026-11-05')
$ErrorActionPreference = 'SilentlyContinue'
$dir = Join-Path $OutDir 'aumentum-capture'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$out = Join-Path $dir ("dbcheck-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format 'yyyyMMdd-HHmm'))
"Aumentum database check on $env:COMPUTERNAME at $(Get-Date)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $dir" -ForegroundColor Red; exit 1 }
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
function Hdr([string]$s) { Log ''; Log "===== $s =====" 'Cyan' }
function Hide([string]$s) { $s -replace '(?i)(password|pwd|encrypted_password)\s*=\s*[^;"]*', '$1=***' }

# 1. The machine
Hdr '1. Machine'
$os = Get-WmiObject Win32_OperatingSystem
Log ("Last boot: {0}   Now: {1}" -f $os.ConvertToDateTime($os.LastBootUpTime), (Get-Date))
Get-WmiObject Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object {
  $free = [math]::Round($_.FreeSpace / 1GB, 1); $size = [math]::Round($_.Size / 1GB, 1)
  $c = if ($_.FreeSpace -lt 2GB) { 'Red' } else { 'Gray' }
  Log ("Disk {0} {1} GB free of {2} GB" -f $_.DeviceID, $free, $size) $c
}
Get-WmiObject Win32_NetworkAdapterConfiguration -Filter 'IPEnabled=True' | ForEach-Object {
  Log ("IP {0}  DNS {1}" -f ($_.IPAddress -join ','), ($_.DNSServerSearchOrder -join ','))
}

# 2. Services that Aumentum depends on
Hdr '2. Services (SQL Server, ArcGIS, ArcSDE, IIS, licensing)'
Get-Service | Where-Object { $_.Name -match '(?i)mssql|sqlserver|sqlbrowser|sqlagent|arcgis|esri|sde|giomgr|w3svc|^was$|hasp|flexlm|lmgrd|aumentum|ils' -or $_.DisplayName -match '(?i)sql server|arcgis|arcsde|aumentum|license' } |
  Sort-Object Status, Name | ForEach-Object {
    $start = (Get-WmiObject Win32_Service -Filter "Name='$($_.Name)'").StartMode
    $c = if ($_.Status -ne 'Running' -and $start -eq 'Auto') { 'Red' } else { 'Gray' }
    Log ("{0,-8} {1,-8} {2}  ({3})" -f $_.Status, $start, $_.DisplayName, $_.Name) $c
  }

# 3. Which database: .sde connection files and Aumentum's web.config files
Hdr '3. Geodatabase connections (.sde files: server / instance / database)'
# cmd's dir /s does not descend into junctions (PowerShell 4's Get-ChildItem -Recurse does, and the
# "Application Data" loops under C:\Users overflow its stack).
function Find-Files([string[]]$roots, [string]$pattern) {
  foreach ($r in $roots) { if ($r -and (Test-Path $r)) { cmd /c "dir /s /b /a-d `"$r\$pattern`" 2>nul" } }
}
$roots = @("$env:ProgramData", "$env:SystemDrive\Users", "${env:ProgramFiles}", "${env:ProgramFiles(x86)}", "$env:SystemDrive\inetpub", "$env:SystemDrive\arcgisserver")
$sdeFiles = Find-Files $roots '*.sde' | Where-Object { $_ } | Select-Object -Unique | ForEach-Object { Get-Item -LiteralPath $_ }
$sdeFiles | Select-Object -First 25 | ForEach-Object {
  $b = [IO.File]::ReadAllBytes($_.FullName)
  $text = [Text.Encoding]::Unicode.GetString($b) + ' ' + [Text.Encoding]::ASCII.GetString($b)
  $props = [regex]::Matches($text, '(?i)(SERVER|INSTANCE|DATABASE|DBCLIENT|DB_CONNECTION_PROPERTIES|AUTHENTICATION_MODE|VERSION)[\x00\s]*[^\x00-\x1f]{0,3}([\w.:\\/\-]{2,80})') |
    ForEach-Object { $_.Value -replace '[\x00-\x1f]+', ' ' } | Select-Object -Unique
  Log ("{0}  ({1})" -f $_.FullName, $_.LastWriteTime)
  foreach ($p in $props) { Log "    $(Hide $p)" }
}
if (-not $sdeFiles) { Log 'No .sde files found under the usual folders.' 'Yellow' }

Hdr '3b. Connection strings in Aumentum web.config / *.config files'
$cfgRoots = @("$env:SystemDrive\inetpub", "${env:ProgramFiles}", "${env:ProgramFiles(x86)}")
$cfgFiles = @(Find-Files $cfgRoots 'web.config') + @(Find-Files $cfgRoots '*.exe.config')
$cfgFiles | Where-Object { $_ -match '(?i)aumentum|\\ils|mcportal|applicationserver|lrs|landsystems|multicada' } | Select-Object -Unique | ForEach-Object {
  $f = $_
  Select-String -LiteralPath $f -Pattern '(?i)(data source|server)\s*=|sde:|instance=|connectionString' | Select-Object -First 15 | ForEach-Object {
    $l = Hide $_.Line.Trim(); if ($l.Length -gt 220) { $l = $l.Substring(0, 220) }
    Log ("{0}:{1}  {2}" -f $f, $_.LineNumber, $l)
  }
}

# 4. Can this server reach the candidate databases?
Hdr '4. Network: database hosts and ports'
$targets = @(@('10.10.10.3', 1433), @('10.10.10.3', 5151), @('127.0.0.1', 1433), @('127.0.0.1', 5151), @('10.10.10.4', 5151))
foreach ($t in $targets) {
  $r = Test-NetConnection -ComputerName $t[0] -Port $t[1] -WarningAction SilentlyContinue
  $c = if ($r.TcpTestSucceeded) { 'Green' } else { 'Red' }
  Log ("{0}:{1}  tcp={2}  ping={3}" -f $t[0], $t[1], $r.TcpTestSucceeded, $r.PingSucceeded) $c
}
foreach ($n in 'PRODUCTIONMC', 'AUMENTUM', 'MC') {
  $ip = try { ([Net.Dns]::GetHostAddresses($n) | ForEach-Object { $_.IPAddressToString }) -join ',' } catch { 'NOT RESOLVED' }
  Log ("name {0} -> {1}" -f $n, $(if ($ip) { $ip } else { 'NOT RESOLVED' }))
}

# 5. Ask each SQL Server one harmless question (Windows sign-in, 10 s timeout)
Hdr '5. SQL Server answers? (SELECT @@SERVERNAME and the databases'' state)'
$q = "SET NOCOUNT ON; SELECT @@SERVERNAME AS server_name, CONVERT(varchar(30), SERVERPROPERTY('ProductVersion')) AS version; SELECT name, state_desc FROM sys.databases ORDER BY name;"
foreach ($inst in 'PRODUCTIONMC\PRODUCTIONMC', '.\PRODUCTIONMC', '10.10.10.3', '127.0.0.1') {
  Log "-- $inst" 'White'
  try {
    $cn = New-Object System.Data.SqlClient.SqlConnection("Server=$inst;Integrated Security=True;Connect Timeout=10;Application Name=aumentum-dbcheck")
    $cn.Open()
    $cmd = $cn.CreateCommand(); $cmd.CommandText = $q; $cmd.CommandTimeout = 20
    $rd = $cmd.ExecuteReader()
    do { while ($rd.Read()) { $vals = for ($i = 0; $i -lt $rd.FieldCount; $i++) { $rd.GetValue($i) }; Log ('   ' + ($vals -join ' | ')) 'Green' } } while ($rd.NextResult())
    $cn.Close()
  } catch { Log ('   FAILED: ' + $_.Exception.Message.Split("`n")[0]) 'Red' }
}

# 5b. On the instance that answers: can the parcels be read, and is anything blocking them?
# All read-only: a row count (30 s limit), what is running or waiting, and log space.
function Ask([string]$inst, [string]$label, [string]$sql, [int]$timeout = 30) {
  Log "-- $label" 'White'
  $t0 = Get-Date
  try {
    $cn = New-Object System.Data.SqlClient.SqlConnection("Server=$inst;Integrated Security=True;Connect Timeout=10;Application Name=aumentum-dbcheck")
    $cn.Open()
    $cmd = $cn.CreateCommand(); $cmd.CommandText = $sql; $cmd.CommandTimeout = $timeout
    $rd = $cmd.ExecuteReader(); $n = 0
    do { while ($rd.Read() -and $n -lt 40) { $n++; $vals = for ($i = 0; $i -lt $rd.FieldCount; $i++) { $rd.GetValue($i) }; Log ('   ' + ($vals -join ' | ')) 'Green' } } while ($rd.NextResult())
    $cn.Close()
    Log ("   ({0:N1} s)" -f ((Get-Date) - $t0).TotalSeconds)
  } catch { Log ("   FAILED after {0:N1} s: {1}" -f ((Get-Date) - $t0).TotalSeconds, $_.Exception.Message.Split("`n")[0]) 'Red' }
}
Hdr '5b. Parcel data, blocking and log space (on PRODUCTIONMC\PRODUCTIONMC)'
$db = 'PRODUCTIONMC\PRODUCTIONMC'
Ask $db 'parcel rows (sde.sde.PLATEAU_STATE_PARCELS, read uncommitted)' "SELECT COUNT_BIG(*) FROM sde.sde.PLATEAU_STATE_PARCELS WITH (NOLOCK);"
Ask $db 'parcel rows (normal read, as Aumentum reads)' "SELECT COUNT_BIG(*) FROM sde.sde.PLATEAU_STATE_PARCELS;"
Ask $db 'blocked or long-running requests' @"
SELECT r.session_id, r.blocking_session_id, r.status, r.command, r.wait_type, r.wait_time/1000 AS wait_s,
       r.total_elapsed_time/1000 AS elapsed_s, DB_NAME(r.database_id) AS db, s.host_name, s.program_name, s.login_name
FROM sys.dm_exec_requests r JOIN sys.dm_exec_sessions s ON s.session_id = r.session_id
WHERE r.session_id <> @@SPID AND (r.blocking_session_id <> 0 OR r.total_elapsed_time > 10000)
ORDER BY r.total_elapsed_time DESC;
"@
Ask $db 'sessions holding locks the others wait on (head blockers)' @"
SELECT DISTINCT s.session_id, s.status, s.host_name, s.program_name, s.login_name, s.last_request_start_time, s.open_transaction_count
FROM sys.dm_exec_sessions s
WHERE s.session_id IN (SELECT blocking_session_id FROM sys.dm_exec_requests WHERE blocking_session_id <> 0)
  AND s.session_id NOT IN (SELECT session_id FROM sys.dm_exec_requests WHERE blocking_session_id <> 0);
"@
Ask $db 'connections per program' "SELECT program_name, host_name, COUNT(*) AS sessions FROM sys.dm_exec_sessions WHERE is_user_process = 1 GROUP BY program_name, host_name ORDER BY sessions DESC;"
Ask $db 'transaction log space used (%)' "DBCC SQLPERF(LOGSPACE);"
Ask $db 'sde database state and recovery' "SELECT name, state_desc, recovery_model_desc, log_reuse_wait_desc, is_read_only FROM sys.databases WHERE name IN ('sde','SDE');"

# 6. What the event logs say since $Since
Hdr "6. System events since $($Since.ToString('yyyy-MM-dd')) (shutdowns, restarts, service failures, disk)"
Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $Since; Id = 6005, 6006, 6008, 1074, 41, 7000, 7001, 7009, 7011, 7022, 7023, 7024, 7031, 7034, 7, 11, 51, 153, 2013 } -ErrorAction SilentlyContinue |
  Select-Object -First 80 | ForEach-Object { Log ("{0}  id={1,-5} {2}  {3}" -f $_.TimeCreated, $_.Id, $_.ProviderName, (($_.Message -split "`n")[0]).Trim()) }

Hdr "6b. Application errors since $($Since.ToString('yyyy-MM-dd')) (SQL Server, ASP.NET, ArcGIS)"
Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $Since; Level = 1, 2, 3 } -ErrorAction SilentlyContinue |
  Where-Object { $_.ProviderName -match '(?i)mssql|sql|asp\.net|arcgis|esri|sde|w3svc|was|\.net runtime|application error' } |
  Select-Object -First 60 | ForEach-Object {
    $m = ($_.Message -replace '\s+', ' ').Trim(); if ($m.Length -gt 400) { $m = $m.Substring(0, 400) + ' ...' }
    Log ("{0}  {1} id={2}  {3}" -f $_.TimeCreated, $_.ProviderName, $_.Id, (Hide $m))
  }

# 7. SQL Server's own error logs and ArcGIS Server's logs
Hdr '7. SQL Server ERRORLOG (errors and the last lines)'
Find-Files @("${env:ProgramFiles}\Microsoft SQL Server", "${env:ProgramFiles(x86)}\Microsoft SQL Server") 'ERRORLOG' | Where-Object { $_ } | ForEach-Object { Get-Item -LiteralPath $_ } | ForEach-Object {
  Log ("{0}  ({1})" -f $_.FullName, $_.LastWriteTime) 'White'
  $lines = Get-Content $_.FullName -ErrorAction SilentlyContinue
  $lines | Where-Object { $_ -match '(?i)error|fail|stopp|shut|insufficient|full|corrupt|suspect' } | Select-Object -Last 25 | ForEach-Object { Log "   ! $_" 'Yellow' }
  $lines | Select-Object -Last 15 | ForEach-Object { Log "     $_" }
}

Hdr '7b. ArcGIS Server logs (SEVERE / WARNING, newest)'
Find-Files @("$env:SystemDrive\arcgisserver\logs") '*.log' | Where-Object { $_ } | ForEach-Object { Get-Item -LiteralPath $_ } |
  Where-Object { $_.LastWriteTime -ge $Since } | Sort-Object LastWriteTime -Descending | Select-Object -First 4 | ForEach-Object {
    Log ("{0}  ({1})" -f $_.FullName, $_.LastWriteTime) 'White'
    Select-String -Path $_.FullName -Pattern 'type="SEVERE"|type="WARNING"' | Select-Object -Last 12 | ForEach-Object {
      $l = ($_.Line -replace '\s+', ' '); if ($l.Length -gt 350) { $l = $l.Substring(0, 350) + ' ...' }; Log "   $l"
    }
  }

# 8. IIS application pools
Hdr '8. IIS application pools'
$appcmd = "$env:windir\System32\inetsrv\appcmd.exe"
if (Test-Path $appcmd) { & $appcmd list apppool | ForEach-Object { Log $_ } } else { Log 'appcmd not found' 'Yellow' }

Log ''
Log "Done. Report: $out" 'Green'
