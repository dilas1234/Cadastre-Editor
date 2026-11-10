<#
  AUMENTUM SDE CONNECTION CHECK - READ-ONLY. Run on PRODUCTIONMC (10.10.10.6):
    powershell -NoProfile -ExecutionPolicy Bypass -File \\tsclient\inventory\aumentum-sdecheck.ps1 -OutDir \\tsclient\inventory
  Why: jobcheck 2026-11-10 16:59 shows GetParcelAttributesByUPIN failing inside
  DataModelHelper.Connect -> SdeWorkspaceFactoryClass.Open: Aumentum cannot open the ArcSDE
  workspace. sde.SDE_process_information and SDE_state_locks both hold exactly 64 rows, which
  looks like the ArcSDE connection limit. This prints the full error above that stack trace and
  the ArcSDE connection limit and its users. Changes nothing. Passwords are never printed.
#>
param([string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path))
$ErrorActionPreference = 'SilentlyContinue'
$dir = Join-Path $OutDir 'aumentum-capture'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$out = Join-Path $dir ("sdecheck-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format 'yyyyMMdd-HHmm'))
"Aumentum SDE connection check on $env:COMPUTERNAME at $(Get-Date)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $dir" -ForegroundColor Red; exit 1 }
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
function Hdr([string]$s) { Log ''; Log "===== $s =====" 'Cyan' }
function Hide([string]$s) { $s -replace '(?i)(password|pwd|encrypted_password)\s*=\s*[^;"<]*', '$1=***' }

Hdr '1. The full error before "SdeWorkspaceFactoryClass.Open" (Server.txt, newest 2)'
$log = "${env:ProgramFiles(x86)}\AUMENTUM\CADASTRE\Server\Logs\Server.txt"
Select-String -Path $log -Pattern 'SdeWorkspaceFactoryClass\.Open' -Context 30, 2 | Select-Object -Last 2 | ForEach-Object {
  Log ("--- line {0}" -f $_.LineNumber) 'White'
  $_.Context.PreContext | ForEach-Object { Log ('   ' + (Hide $_)) }
  Log ('>  ' + (Hide $_.Line)) 'Yellow'
  $_.Context.PostContext | ForEach-Object { Log ('   ' + (Hide $_)) }
}
Hdr '1b. When it started: first SdeWorkspaceFactoryClass.Open error in each log since Nov 1'
Get-ChildItem "${env:ProgramFiles(x86)}\AUMENTUM\CADASTRE\Server\Logs" -Filter 'Server*.txt' | Where-Object { $_.LastWriteTime -ge '2026-11-01' } | Sort-Object LastWriteTime | ForEach-Object {
  $first = Select-String -Path $_.FullName -Pattern 'SdeWorkspaceFactoryClass\.Open' | Select-Object -First 1
  $count = (Select-String -Path $_.FullName -Pattern 'SdeWorkspaceFactoryClass\.Open' | Measure-Object).Count
  $ok = (Select-String -Path $_.FullName -Pattern 'GetParcelAttributesByUPIN' | Measure-Object).Count
  Log ("{0}: {1} Open errors, {2} UPIN lookup lines, first error at line {3}" -f $_.Name, $count, $ok, $(if ($first) { $first.LineNumber } else { '-' }))
}

function Ask([string]$label, [string]$sql) {
  Log "-- $label" 'White'
  try {
    $cn = New-Object System.Data.SqlClient.SqlConnection("Server=PRODUCTIONMC\PRODUCTIONMC;Database=sde;Integrated Security=True;Connect Timeout=10;Application Name=aumentum-sdecheck")
    $cn.Open()
    $cmd = $cn.CreateCommand(); $cmd.CommandText = $sql; $cmd.CommandTimeout = 60
    $rd = $cmd.ExecuteReader(); $n = 0
    do { while ($rd.Read() -and $n -lt 80) { $n++; $vals = for ($i = 0; $i -lt $rd.FieldCount; $i++) { $rd.GetValue($i) }; Log ('   ' + ($vals -join ' | ')) 'Green' } } while ($rd.NextResult())
    $cn.Close()
  } catch { Log ('   FAILED: ' + $_.Exception.Message.Split("`n")[0]) 'Red' }
}
Hdr '2. ArcSDE connection limit and who holds the connections'
Ask 'server_config: connection settings' "SELECT prop_name, num_prop_value, char_prop_value FROM sde.SDE_server_config WHERE prop_name LIKE '%CONN%' OR prop_name IN ('MAXSTANDBY','TCPKEEPALIVE','MINBUFSIZE','MAXBUFSIZE');"
Ask 'process_information: count, oldest, newest' "SELECT COUNT(*), MIN(start_time), MAX(start_time) FROM sde.SDE_process_information;"
Ask 'process_information by user / machine / program' "SELECT owner, nodename, direct_connect, COUNT(*), MIN(start_time), MAX(start_time) FROM sde.SDE_process_information GROUP BY owner, nodename, direct_connect ORDER BY COUNT(*) DESC;"
Ask 'process_information whose SQL session is gone (orphans)' "SELECT COUNT(*) FROM sde.SDE_process_information p WHERE NOT EXISTS (SELECT 1 FROM sys.dm_exec_sessions s WHERE s.session_id = p.spid);"
Ask 'state_locks by owner process' "SELECT TOP 20 sde_id, COUNT(*) FROM sde.SDE_state_locks GROUP BY sde_id ORDER BY COUNT(*) DESC;"

Hdr '3. gsrvr.exe processes (one per ArcSDE application-server connection)'
$g = Get-WmiObject Win32_Process -Filter "Name='gsrvr.exe'"
Log ("gsrvr.exe running: {0}" -f @($g).Count)
$g | Sort-Object CreationDate | Select-Object -First 5 | ForEach-Object { Log ("   pid {0} started {1}" -f $_.ProcessId, $_.ConvertToDateTime($_.CreationDate)) }

Hdr '4. ArcSDE service log (sde_sde.log / giomgr), last 40 lines'
Get-ChildItem "$env:ProgramFiles\ArcGIS\ArcSDE\sqlexe\etc" -Filter '*.log' | Sort-Object LastWriteTime -Descending | Select-Object -First 2 | ForEach-Object {
  Log ("{0}  ({1})" -f $_.FullName, $_.LastWriteTime) 'White'
  Get-Content $_.FullName -Tail 40 | ForEach-Object { Log ('   ' + (Hide $_)) }
}

Log ''
Log "Done. Report: $out" 'Green'
