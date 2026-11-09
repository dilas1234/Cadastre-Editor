<#
  CHARTING CAPTURE (docs/charting-and-reporting.docx, phase C0) - READ-ONLY on this server.
  Run inside the Remote Desktop session on PRODUCTIONMC (RUN-ME-CHARTING.cmd). Writes only to
  <OutDir>\aumentum-capture\charting\ (back on the Linux machine via \\tsclient\inventory):
    mxd\       every Cadastral_Maps*.mxd
    paths-*.txt every path and raster name read out of each map document
    sheets\    the scans those documents use, with world files and other sidecars
    gdb\       the file geodatabases they use (Willams, JGOMOS, ...)
    form\      Survey Charting Report.html
    manifest.txt  what was found, copied, missing or skipped
  An .mxd keeps a raster's folder and its file name apart, so both are read and put together.
  Stops copying at -MaxGB. PowerShell 4 compatible (Windows Server 2012 R2).
#>
param([string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path), [int]$MaxGB = 60)
$ErrorActionPreference = 'SilentlyContinue'
$cap = Join-Path $OutDir 'aumentum-capture\charting'
foreach ($d in 'mxd', 'sheets', 'gdb', 'form') { New-Item -ItemType Directory -Path (Join-Path $cap $d) -Force | Out-Null }
$log = Join-Path $cap 'manifest.txt'
"Charting capture of $env:COMPUTERNAME started $(Get-Date)" | Out-File $log -Encoding UTF8
if (-not (Test-Path $log)) { Write-Host "Cannot write to $cap" -ForegroundColor Red; exit 1 }
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $log -Append -Encoding UTF8 }

$script:copied = [long]0
$budget = [long]$MaxGB * 1GB
$rasterExt = 'tif|tiff|jpg|jpeg|img|sid|ecw|jp2|png|bmp'

function Copy-Budgeted([IO.FileInfo]$f, [string]$dest) {
  if ($script:copied + $f.Length -gt $budget) { Log "SKIPPED (over ${MaxGB} GB): $($f.FullName)" 'Yellow'; return $false }
  New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
  Copy-Item -LiteralPath $f.FullName -Destination $dest -Force
  if (Test-Path -LiteralPath $dest) { $script:copied += $f.Length; return $true }
  Log "COULD NOT COPY: $($f.FullName)" 'Yellow'; return $false
}
# D:\Maps\a.tif -> <root>\D\Maps\a.tif ; \\HOST\Share\a.tif -> <root>\HOST\Share\a.tif
function Local-Path([string]$root, [string]$full) {
  Join-Path $root (($full -replace '^\\\\', '') -replace ':', '')
}
# Text in an .mxd is UTF-16, at either byte alignment, or plain ANSI.
function Mxd-Text([string]$file) {
  $b = [IO.File]::ReadAllBytes($file)
  [Text.Encoding]::Unicode.GetString($b, 0, $b.Length)
  [Text.Encoding]::Unicode.GetString($b, 1, $b.Length - 1)
  [Text.Encoding]::Default.GetString($b)
}

Log "--- Map documents" 'Cyan'
$mxds = @()
foreach ($p in 'C:\Users\*\Desktop\Cadastral_Maps*.mxd', 'C:\Users\*\Documents\Cadastral_Maps*.mxd', 'C:\Users\*\Cadastral_Maps*.mxd',
               'E:\Profile\*\Desktop\Cadastral_Maps*.mxd', 'E:\Profile\*\Documents\Cadastral_Maps*.mxd',
               'C:\Cadastral_Maps*.mxd', 'D:\Cadastral_Maps*.mxd', 'D:\*\Cadastral_Maps*.mxd', 'D:\*\*\Cadastral_Maps*.mxd', 'E:\*\Cadastral_Maps*.mxd') {
  $mxds += @(Get-ChildItem -Path $p -File)
}
$mxds = @($mxds | Sort-Object FullName -Unique | Sort-Object LastWriteTime -Descending)
if (-not $mxds.Count) { Log "NO Cadastral_Maps*.mxd FOUND" 'Red' }

$paths = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
$names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
# Binary sits right before each string, so only an ASCII letter or digit rules a match out.
$pathRe = [regex]'(?i)(?<![a-z0-9])(?:[a-z]:\\|\\\\[a-z0-9_.\-]+\\)[^\x00-\x1f"<>|*?\uFFFD]{1,240}'
$nameRe = [regex]"(?i)[a-z0-9][a-z0-9_ .()\-]{0,120}\.(?:$rasterExt)(?![a-z0-9])"
$texts = New-Object 'System.Collections.Generic.List[string]'
$i = 0
foreach ($m in $mxds) {
  $i++
  $owner = ($m.FullName -split '\\' | Select-Object -Skip 2 -First 1)
  $dest = Join-Path $cap ("mxd\{0}__{1}" -f $owner, $m.Name)
  Copy-Item -LiteralPath $m.FullName -Destination $dest -Force
  Log ("mxd {0} ({1:N1} MB, {2})" -f $m.FullName, ($m.Length / 1MB), $m.LastWriteTime)
  $here = New-Object 'System.Collections.Generic.List[string]'
  foreach ($t in Mxd-Text $m.FullName) {
    $texts.Add($t)
    foreach ($x in $pathRe.Matches($t)) { $v = $x.Value.Trim(); [void]$paths.Add($v); $here.Add($v) }
    foreach ($x in $nameRe.Matches($t)) { $v = $x.Value.Trim(); [void]$names.Add($v); $here.Add($v) }
  }
  $here | Sort-Object -Unique | Out-File (Join-Path $cap "paths-$i.txt") -Encoding UTF8
}
Log "$($paths.Count) paths and $($names.Count) raster names read from $($mxds.Count) map documents"

Log "--- The form" 'Cyan'
foreach ($p in 'C:\Survey Charting Report*.htm*', 'C:\*\Survey Charting Report*.htm*', 'C:\Users\*\Desktop\*Charting*Report*.htm*', 'C:\Users\*\Documents\*Charting*Report*.htm*', 'E:\Profile\*\Desktop\*Charting*Report*.htm*', 'D:\*\*Charting*Report*.htm*') {
  foreach ($f in @(Get-ChildItem -Path $p -File)) {
    Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $cap ("form\" + $f.Name)) -Force
    Log "form $($f.FullName)"
  }
}

Log "--- Scans" 'Cyan'
# Folders the documents point at: the path itself when it is a folder, its parent when it is a file.
$dirs = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
$gdbs = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($p in $paths) {
  $g = [regex]::Match($p, '(?i)^.*?\.gdb')
  if ($g.Success) { [void]$gdbs.Add($g.Value); continue }
  if (Test-Path -LiteralPath $p -PathType Container) { [void]$dirs.Add($p.TrimEnd('\')) }
  elseif (Test-Path -LiteralPath $p -PathType Leaf) { [void]$dirs.Add((Split-Path $p -Parent)) }
  else { Log "not found here: $p" 'DarkGray' }
}
# A raster counts if its real file name appears in a document: stray bytes can run into a name
# read out of the binary, so the names on disk are looked for, not the other way round.
function In-Documents([string]$name) {
  foreach ($t in $texts) { if ($t.IndexOf($name, [StringComparison]::OrdinalIgnoreCase) -ge 0) { return $true } }
  return $false
}
$found = 0
foreach ($d in $dirs) {
  foreach ($f in @(Get-ChildItem -LiteralPath $d -File | Where-Object { $_.Extension -match "^\.($rasterExt)$" -and (In-Documents $_.Name) })) {
    if (Copy-Budgeted $f (Local-Path (Join-Path $cap 'sheets') $f.FullName)) {
      $found++
      # World file, pyramids, statistics, projection: everything with this base name.
      foreach ($s in @(Get-ChildItem -LiteralPath $d -File -Filter ($f.BaseName + '.*') | Where-Object { $_.FullName -ne $f.FullName })) {
        [void](Copy-Budgeted $s (Local-Path (Join-Path $cap 'sheets') $s.FullName))
      }
      Log ("sheet {0} ({1:N1} MB)" -f $f.FullName, ($f.Length / 1MB))
    }
  }
}
Log "$found scans copied"

Log "--- File geodatabases" 'Cyan'
foreach ($g in $gdbs) {
  if (-not (Test-Path -LiteralPath $g -PathType Container)) { Log "not found here: $g" 'Yellow'; continue }
  $files = @(Get-ChildItem -LiteralPath $g -File -Recurse | Where-Object { $_.Extension -ne '.lock' })
  $size = ($files | Measure-Object Length -Sum).Sum
  Log ("gdb {0} ({1} files, {2:N1} MB)" -f $g, $files.Count, ($size / 1MB))
  foreach ($f in $files) { [void](Copy-Budgeted $f (Local-Path (Join-Path $cap 'gdb') $f.FullName)) }
}

# Nothing from the documents: list where world files live, so the right folder can be named.
if (-not $found) {
  Log "--- No scans from the documents: looking for world files on D: and E: (top levels only)" 'Cyan'
  foreach ($root in 'D:\', 'E:\') { foreach ($lvl in '', '*\', '*\*\', '*\*\*\') { foreach ($e in '*.tfw', '*.jgw') {
    foreach ($f in @(Get-ChildItem -Path (Join-Path $root ($lvl + $e)) -File | Select-Object -First 200)) { Log "world file $($f.FullName)" }
  } } }
}
Log ("Finished {0}: {1:N2} GB copied to {2}" -f (Get-Date), ($script:copied / 1GB), $cap) 'Green'
