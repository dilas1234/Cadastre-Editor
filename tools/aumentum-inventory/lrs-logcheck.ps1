<#
  LRS JBOSS LOG CHECK - READ-ONLY. Run inside the Remote Desktop session on the REGISTRY server
  via RUN-ME-LOGCHECK.cmd. Changes nothing: it only reads C:\LRSAS\server\default\log and lists
  D:\LRS_STORAGE\lucene-indexes. Safe to run while LRSAS is running, during working hours.
  Why: the inventory of 2026-11-02 found 5566 files in the log folder but captured none of them.
  This summarises the folder, shows the tail of the live server.log/boot.log, the last startups,
  and every Lucene / index / lock / memory problem since -Since.
  Writes <OutDir>\lrs-capture\logcheck-<host>-<time>.txt. Passwords are masked.
  PowerShell 3+ (Windows Server 2012 R2 has 4).
#>
param([string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path),
      [datetime]$Since = (Get-Date).AddDays(-30),
      [string]$LogDir = 'C:\LRSAS\server\default\log',
      [string]$IndexDir = 'D:\LRS_STORAGE\lucene-indexes')
$ErrorActionPreference = 'SilentlyContinue'
$dir = Join-Path $OutDir 'lrs-capture'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$out = Join-Path $dir ("logcheck-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format 'yyyyMMdd-HHmm'))
"LRS JBoss log check on $env:COMPUTERNAME at $(Get-Date)   (since $Since)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $dir" -ForegroundColor Red; exit 1 }
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
function Hdr([string]$s) { Log ''; Log "===== $s =====" 'Cyan' }
function Hide([string]$s) { $s -replace '(?i)(password|passwd|pwd|secret)(\s*[=:]\s*|">\s*)[^;"<\s,]*', '$1$2***' }
function Mb($b) { '{0:N1} MB' -f ($b / 1MB) }

# 1. Service and disk
Hdr '1. Service, Java process, disks'
Get-Service LRSAS | ForEach-Object { Log ("{0}  {1}  {2}" -f $_.Name, $_.Status, $_.DisplayName) }
Get-Process java | ForEach-Object { Log ("java pid {0}  started {1}  working set {2}  {3}" -f $_.Id, $_.StartTime, (Mb $_.WorkingSet64), $_.Path) }
Get-WmiObject Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object { Log ("{0}  free {1:N1} GB of {2:N1} GB" -f $_.DeviceID, ($_.FreeSpace/1GB), ($_.Size/1GB)) }

# 2. Log folder summary
Hdr "2. Log folder $LogDir"
$all = @(Get-ChildItem $LogDir -File -ErrorAction SilentlyContinue)
if ($all.Count -eq 0) { $all = @(Get-ChildItem $LogDir | Where-Object { -not $_.PSIsContainer }) }
$sum = ($all | Measure-Object Length -Sum).Sum
Log ("{0} files, {1} total" -f $all.Count, (Mb $sum))
if ($all.Count) {
  $byAge = $all | Sort-Object LastWriteTime
  Log ("oldest: {0}  {1}" -f $byAge[0].Name, $byAge[0].LastWriteTime)
  Log ("newest: {0}  {1}" -f $byAge[-1].Name, $byAge[-1].LastWriteTime)
  Log ''
  Log 'Files by name pattern (digits replaced by #):'
  $all | Group-Object { $_.Name -replace '\d', '#' } | Sort-Object Count -Descending | Select-Object -First 15 |
    ForEach-Object { Log ("{0,6}  {1,12}  {2}" -f $_.Count, (Mb (($_.Group | Measure-Object Length -Sum).Sum)), $_.Name) }
  Log ''
  Log '25 newest files:'
  $all | Sort-Object LastWriteTime -Descending | Select-Object -First 25 |
    ForEach-Object { Log ("{0}  {1,12}  {2}" -f $_.LastWriteTime, (Mb $_.Length), $_.Name) }
}

# 3. Tail of the live logs
foreach ($n in 'boot.log', 'server.log') {
  $f = Join-Path $LogDir $n
  Hdr "3. Last 150 lines of $n"
  if (Test-Path $f) {
    $i = Get-Item $f; Log ("{0}  {1}" -f $i.LastWriteTime, (Mb $i.Length))
    Get-Content $f -Tail 150 | ForEach-Object { Log (Hide $_) }
  } else { Log 'not found' }
}

# Files in the period: the live logs plus rolled ones written since -Since
$recent = @($all | Where-Object { $_.LastWriteTime -ge $Since -and $_.Name -match '(?i)\.log|log\.' } | Sort-Object LastWriteTime)
Hdr "4. Startups and shutdowns in $($recent.Count) log files written since $Since"
$recent | Select-String -Pattern 'Started in|JBoss .* Started|Shutting down|Shutdown complete|Starting JBoss' -SimpleMatch:$false |
  Select-Object -Last 40 | ForEach-Object { Log ("{0}: {1}" -f $_.Filename, $_.Line.Trim()) }

# 5. Lucene / index / lock / memory problems
Hdr '5. Lucene, index, lock and memory problems (counts)'
$pat = 'lucene|LockObtainFailed|write\.lock|CorruptIndex|IndexNotFound|segments|reindex|re-index|rebuild.*index|index.*(fail|error|corrupt|lock|recover)|OutOfMemory|Java heap space|GC overhead|Too many open files'
$hits = @($recent | Select-String -Pattern $pat)
Log ("{0} matching lines" -f $hits.Count)
$hits | Group-Object { ([regex]::Match($_.Line, '(?i)' + $pat)).Value.ToLower() } | Sort-Object Count -Descending |
  ForEach-Object { Log ("{0,7}  {1}" -f $_.Count, $_.Name) }
Hdr '5b. Distinct messages of those lines (timestamps stripped, max 80, first seen)'
$seen = @{}
foreach ($h in $hits) {
  $key = ($h.Line -replace '^\S+ \S+\s+', '' -replace '\d+', '#').Trim()
  if ($key.Length -gt 220) { $key = $key.Substring(0, 220) }
  if ($seen.ContainsKey($key)) { $seen[$key].n++ } else { $seen[$key] = @{ n = 1; first = "$($h.Filename): $($h.Line.Trim())" } }
}
$seen.GetEnumerator() | Sort-Object { $_.Value.n } -Descending | Select-Object -First 80 |
  ForEach-Object { Log ("[x{0}] {1}" -f $_.Value.n, (Hide $_.Value.first)) }
Hdr '5c. Last 60 matching lines with 3 lines of context'
$recent | Select-String -Pattern $pat -Context 1, 3 | Select-Object -Last 60 | ForEach-Object {
  Log ("--- {0}:{1}" -f $_.Filename, $_.LineNumber)
  $_.Context.PreContext | ForEach-Object { Log ('   ' + (Hide $_)) }
  Log ('>> ' + (Hide $_.Line))
  $_.Context.PostContext | ForEach-Object { Log ('   ' + (Hide $_)) }
}

# 6. ERROR / FATAL by source class
Hdr '6. ERROR and FATAL lines by class (top 30)'
$errs = @($recent | Select-String -Pattern '\s(ERROR|FATAL)\s')
Log ("{0} ERROR/FATAL lines" -f $errs.Count)
$errs | Group-Object { $m = [regex]::Match($_.Line, '\s(ERROR|FATAL)\s+\[([^\]]+)\]'); if ($m.Success) { $m.Groups[2].Value } else { '(no class)' } } |
  Sort-Object Count -Descending | Select-Object -First 30 | ForEach-Object { Log ("{0,7}  {1}" -f $_.Count, $_.Name) }
Hdr '6b. Last 40 ERROR/FATAL lines'
$errs | Select-Object -Last 40 | ForEach-Object { Log ("{0}: {1}" -f $_.Filename, (Hide $_.Line.Trim())) }

# 7. Index folder (list only)
Hdr "7. Index folder $IndexDir (listing only)"
if (Test-Path $IndexDir) {
  Get-ChildItem $IndexDir | Where-Object { $_.PSIsContainer } | ForEach-Object {
    $fs = @(Get-ChildItem $_.FullName -Recurse | Where-Object { -not $_.PSIsContainer })
    $newest = $fs | Sort-Object LastWriteTime | Select-Object -Last 1
    Log ("{0,-40} {1,6} files {2,12}  newest {3}" -f $_.Name, $fs.Count, (Mb (($fs | Measure-Object Length -Sum).Sum)), $newest.LastWriteTime)
  }
  Log ''
  Log 'Lock files (write.lock etc.):'
  Get-ChildItem $IndexDir -Recurse | Where-Object { $_.Name -match '(?i)\.lock$|^lock' } |
    ForEach-Object { Log ("{0}  {1}" -f $_.LastWriteTime, $_.FullName) }
} else { Log 'not found' }

Hdr 'END'
Log "Finished $(Get-Date). Report: $out" 'Green'
