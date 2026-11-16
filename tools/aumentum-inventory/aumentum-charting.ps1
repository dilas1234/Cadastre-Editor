<#
  CHARTING CAPTURE (docs/charting-and-reporting.docx, phase C0) - READ-ONLY on this server.
  Run inside the Remote Desktop session on PRODUCTIONMC (RUN-ME-CHARTING.cmd). Writes only to
  <OutDir>\aumentum-capture\charting\ (back on the Linux machine via \\tsclient\inventory):
    mxd\          every Cadastral_Maps*.mxd
    paths-*.txt   every path and raster name read out of each map document
    sheets\       the scans those documents use, with their sidecars (world file, .aux.xml, .prj, ...)
    georef\       world files / .aux.xml / .prj found anywhere under the sheet folders and the
                  Image share, for scans whose own TIFF tags do not place them (see below)
    gdb\          the file geodatabases: the documents' own, every *.gdb under the Image share
                  and the officers' Default.gdb (Willams, JGOMOS, Maffy_*, Jgomos_* live in one of them)
    form\         Survey Charting Report.html
    georef.txt    per scan: how it is placed (world file, .aux.xml, GeoTIFF tags only, nothing)
    chartingMinna.txt  the chartingMinna database's tables and row counts (structure only)
    manifest.txt  what was found, copied, missing or skipped
  Second run (6 Nov 2026 stopped after 14 scans): a file already copied with the same size is not
  copied again, so a re-run carries on where the last one stopped.
  The 14 scans copied had GeoTIFF tags placing them at 0,0 (pixel 0.005, Minna datum) and only
  .ovr sidecars, so their real placement is somewhere else: this run looks for it.
  Stops copying at -MaxGB. PowerShell 4 compatible (Windows Server 2012 R2).
#>
param([string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path), [int]$MaxGB = 60)
$ErrorActionPreference = 'SilentlyContinue'
$cap = Join-Path $OutDir 'aumentum-capture\charting'
foreach ($d in 'mxd', 'sheets', 'georef', 'gdb', 'form') { New-Item -ItemType Directory -Path (Join-Path $cap $d) -Force | Out-Null }
$log = Join-Path $cap 'manifest.txt'
"Charting capture of $env:COMPUTERNAME started $(Get-Date)" | Out-File $log -Encoding UTF8
if (-not (Test-Path $log)) { Write-Host "Cannot write to $cap" -ForegroundColor Red; exit 1 }
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $log -Append -Encoding UTF8 }
$geoLog = Join-Path $cap 'georef.txt'
"How each scan is placed ($(Get-Date))" | Out-File $geoLog -Encoding UTF8

$script:copied = [long]0
$script:reused = 0
$budget = [long]$MaxGB * 1GB
$rasterExt = 'tif|tiff|jpg|jpeg|img|sid|ecw|jp2|png|bmp'
$sidecarExt = '\.(tfw|tifw|tiffw|wld|jgw|jpgw|sdw|j2w|aux\.xml|aux|prj|xml|rrd|ovr|tif\.xml)$'

function Copy-Budgeted([IO.FileInfo]$f, [string]$dest) {
  # Already there from an earlier run: keep it.
  $have = Get-Item -LiteralPath $dest
  if ($have -and $have.Length -eq $f.Length) { $script:reused++; return $true }
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
# Folders under $root down to $depth levels (the root included).
function Folders-Under([string]$root, [int]$depth) {
  $out = @($root); $level = @($root)
  for ($i = 0; $i -lt $depth; $i++) {
    $level = @($level | ForEach-Object { Get-ChildItem -LiteralPath $_ -Directory | Where-Object { $_.Name -notmatch '\.gdb$' } | ForEach-Object { $_.FullName } })
    if (-not $level.Count) { break }
    $out += $level
  }
  $out
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
# A raster layer is stored as "<folder>\; RasterDataset = <name>;": put the two together.
$rasterRe = [regex]'(?i)^(?<dir>.+?\\)\s*;\s*RasterDataset\s*=\s*(?<name>[^;]+?)\s*;?\s*$'
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
    foreach ($x in $pathRe.Matches($t)) {
      $v = $x.Value.Trim()
      $r = $rasterRe.Match($v)
      if ($r.Success) { $v = $r.Groups['dir'].Value + $r.Groups['name'].Value; [void]$names.Add($r.Groups['name'].Value) }
      $v = $v.TrimEnd(';', ' ')
      [void]$paths.Add($v); $here.Add($v)
    }
    foreach ($x in $nameRe.Matches($t)) { $v = $x.Value.Trim(); [void]$names.Add($v); $here.Add($v) }
  }
  $here | Sort-Object -Unique | Out-File (Join-Path $cap "paths-$i.txt") -Encoding UTF8
}
Log "$($paths.Count) paths and $($names.Count) raster names read from $($mxds.Count) map documents"

Log "--- The form" 'Cyan'
$forms = 0
foreach ($p in 'C:\Survey Charting Report*.htm*', 'C:\*\Survey Charting Report*.htm*', 'C:\Users\*\Desktop\*Charting*Report*.htm*', 'C:\Users\*\Documents\*Charting*Report*.htm*',
               'C:\Users\*\Desktop\*Survey*Report*.htm*', 'C:\Users\*\Downloads\*Survey*Report*.htm*', 'E:\Profile\*\Desktop\*Charting*Report*.htm*',
               'E:\Profile\*\Desktop\*Survey*Report*.htm*', 'E:\Profile\*\Documents\*Survey*Report*.htm*', 'D:\*\*Charting*Report*.htm*', 'D:\*\*Survey*Report*.htm*') {
  foreach ($f in @(Get-ChildItem -Path $p -File)) {
    Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $cap ("form\" + $f.Name)) -Force
    # The page's images (heading box) sit in a "<name>_files" folder beside it.
    $assets = Join-Path $f.DirectoryName ($f.BaseName + '_files')
    if (Test-Path -LiteralPath $assets) { Copy-Item -LiteralPath $assets -Destination (Join-Path $cap 'form') -Recurse -Force }
    Log "form $($f.FullName)"; $forms++
  }
}
if (-not $forms) { Log "form NOT FOUND on $env:COMPUTERNAME: the recording opened file:///C:/Survey Charting Report.html on the charting PC; copy it from there by hand" 'Yellow' }

Log "--- Scans" 'Cyan'
# Folders the documents point at: the path itself when it is a folder, its parent when it is a file.
# Their subfolders count too (the documents also use Minna Datum_Scanned_files\3_4).
$dirs = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
$gdbs = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
$missing = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($p in $paths) {
  $g = [regex]::Match($p, '(?i)^.*?\.gdb')
  if ($g.Success) { [void]$gdbs.Add($g.Value); continue }
  if (Test-Path -LiteralPath $p -PathType Container) { [void]$dirs.Add($p.TrimEnd('\')) }
  elseif (Test-Path -LiteralPath $p -PathType Leaf) { [void]$dirs.Add((Split-Path $p -Parent)) }
  else {
    $parent = Split-Path $p -Parent
    if ($parent -and (Test-Path -LiteralPath $parent -PathType Container)) { [void]$dirs.Add($parent.TrimEnd('\')) }
    [void]$missing.Add($p)
  }
}
foreach ($d in @($dirs)) { foreach ($s in Folders-Under $d 2) { [void]$dirs.Add($s) } }
Log ("{0} sheet folders; {1} paths in the documents not on this server (listed at the end)" -f $dirs.Count, $missing.Count)

# A raster counts if its real file name appears in a document: stray bytes can run into a name
# read out of the binary, so the names on disk are looked for, not the other way round.
function In-Documents([string]$name) {
  foreach ($t in $texts) { if ($t.IndexOf($name, [StringComparison]::OrdinalIgnoreCase) -ge 0) { return $true } }
  return $false
}
# How a scan is placed: its own sidecar, else what its GeoTIFF tags say.
function Placement([IO.FileInfo]$f) {
  $side = @(Get-ChildItem -LiteralPath $f.DirectoryName -File -Filter ($f.BaseName + '.*') | Where-Object { $_.Name -ne $f.Name -and $_.Name -match $sidecarExt -and $_.Name -notmatch '\.ovr$|\.rrd$' })
  if ($side.Count) { return 'sidecar: ' + (($side | ForEach-Object { $_.Name }) -join ', ') }
  if ($f.Extension -notmatch '^\.tiff?$') { return 'no sidecar' }
  # The first TIFF directory (it may sit at the end of a large scan): GeoTIFF tags 33550 / 33922
  # (pixel scale, tie point) or 34264 (transformation).
  try {
    $fs = [IO.File]::OpenRead($f.FullName); $br = New-Object IO.BinaryReader($fs)
    $order = $br.ReadUInt16(); $magic = $br.ReadUInt16()
    if ($order -ne 0x4949) { $fs.Close(); return 'big-endian TIFF, not checked' }
    if ($magic -eq 43) { $fs.Close(); return 'BigTIFF, not checked' }
    $fs.Position = $br.ReadUInt32(); $n = $br.ReadUInt16(); $geo = @(); $tieAt = 0
    for ($k = 0; $k -lt $n; $k++) {
      $tag = $br.ReadUInt16(); [void]$br.ReadUInt16(); [void]$br.ReadUInt32(); $val = $br.ReadUInt32()
      if ($tag -eq 33550 -or $tag -eq 33922 -or $tag -eq 34264) { $geo += $tag }
      if ($tag -eq 33922) { $tieAt = $val }
    }
    $where = ''
    if ($tieAt) { $fs.Position = $tieAt + 24; $where = (' at {0:N2}, {1:N2}' -f $br.ReadDouble(), $br.ReadDouble()) }
    $fs.Close()
    if ($geo.Count) { return "GeoTIFF tags only ($($geo -join ', '))$where" } else { return 'NOTHING: no sidecar, no GeoTIFF tags' }
  } catch { return 'unreadable' }
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
      $how = Placement $f
      ("{0}`t{1}" -f $f.FullName, $how) | Out-File $geoLog -Append -Encoding UTF8
      Log ("sheet {0} ({1:N1} MB) - {2}" -f $f.FullName, ($f.Length / 1MB), $how)
    }
  }
}
Log "$found scans copied or already there ($($script:reused) files kept from the last run)"

Log "--- Georeferencing: world files, .aux.xml and .prj anywhere under the sheet folders and the Image share" 'Cyan'
# The scans' own tags do not place them. ArcMap keeps a georeferenced raster's placement in a
# world file or .aux.xml, possibly beside a rectified copy elsewhere: list and copy every one.
$roots = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($d in $dirs) { [void]$roots.Add($d) }
foreach ($r in '\\PRODUCTIONMC\Image', 'D:\Image', 'E:\Image', 'C:\Users\jgomos\Desktop\geo_reference MAps', 'E:\Profile\jgomos.V2\Desktop\geo_reference MAps', '\\PRODUCTIONMC\Image\Shapefile\utm') {
  if (Test-Path -LiteralPath $r -PathType Container) { [void]$roots.Add($r) }
}
$sheetBases = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
foreach ($n in $names) { [void]$sheetBases.Add([IO.Path]::GetFileNameWithoutExtension($n)) }
$georef = 0
foreach ($r in $roots) {
  foreach ($d in Folders-Under $r 4) {
    foreach ($f in @(Get-ChildItem -LiteralPath $d -File | Where-Object { $_.Name -match '\.(tfw|tifw|tiffw|wld|jgw|aux\.xml|prj)$' -and $_.Length -lt 20MB })) {
      $base = $f.Name -replace '(?i)\.(tif|tiff|jpg|jpeg|img|ecw|sid)?\.?(tfw|tifw|tiffw|wld|jgw|aux\.xml|prj)$', ''
      $base = $base.TrimEnd('.')
      $match = if ($sheetBases.Contains($base)) { 'a sheet of the documents' } else { 'other' }
      if (Copy-Budgeted $f (Local-Path (Join-Path $cap 'georef') $f.FullName)) {
        $georef++
        ("{0}`t{1:N0} bytes`t{2}" -f $f.FullName, $f.Length, $match) | Out-File $geoLog -Append -Encoding UTF8
      }
    }
    # A georeferenced copy of a sheet under another name or folder (rectified, _geo, _utm ...).
    foreach ($f in @(Get-ChildItem -LiteralPath $d -File | Where-Object { $_.Extension -match "^\.($rasterExt)$" -and -not (In-Documents $_.Name) })) {
      foreach ($b in $sheetBases) {
        if ($b.Length -ge 6 -and $f.BaseName.IndexOf($b, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
          ("other copy of {0}: {1} ({2:N1} MB) - {3}" -f $b, $f.FullName, ($f.Length / 1MB), (Placement $f)) | Out-File $geoLog -Append -Encoding UTF8
          break
        }
      }
    }
  }
}
Log "$georef world files / .aux.xml / .prj copied (georef\, listed in georef.txt)"

Log "--- File geodatabases" 'Cyan'
foreach ($p in 'C:\Users\*\Documents\ArcGIS\*.gdb', 'E:\Profile\*\Documents\ArcGIS\*.gdb') { foreach ($g in @(Get-ChildItem -Path $p -Directory)) { [void]$gdbs.Add($g.FullName) } }
foreach ($r in $roots) { foreach ($d in Folders-Under $r 3) { foreach ($g in @(Get-ChildItem -LiteralPath $d -Directory -Filter '*.gdb')) { [void]$gdbs.Add($g.FullName) } } }
foreach ($g in $gdbs) {
  if (-not (Test-Path -LiteralPath $g -PathType Container)) { Log "not found here: $g" 'Yellow'; continue }
  $files = @(Get-ChildItem -LiteralPath $g -File -Recurse | Where-Object { $_.Extension -ne '.lock' })
  $size = ($files | Measure-Object Length -Sum).Sum
  # Feature class names are in the catalog table as UTF-16 text; list the charting ones seen.
  $cat = Join-Path $g 'a00000001.gdbtable'
  $layers = ''
  if (Test-Path -LiteralPath $cat) {
    $t = [Text.Encoding]::Unicode.GetString([IO.File]::ReadAllBytes($cat))
    $layers = (([regex]'(?i)(willams|jgomos[a-z0-9_]*|maffy[a-z0-9_]*|[a-z0-9_]*parcel[a-z0-9_]*|[a-z0-9_]*point[a-z0-9_]*)').Matches($t) | ForEach-Object { $_.Value } | Sort-Object -Unique) -join ', '
  }
  Log ("gdb {0} ({1} files, {2:N1} MB){3}" -f $g, $files.Count, ($size / 1MB), $(if ($layers) { " - layers: $layers" } else { '' }))
  foreach ($f in $files) { [void](Copy-Budgeted $f (Local-Path (Join-Path $cap 'gdb') $f.FullName)) }
}

Log "--- chartingMinna database (structure only)" 'Cyan'
$out = Join-Path $cap 'chartingMinna.txt'
"chartingMinna: tables and row counts ($(Get-Date))" | Out-File $out -Encoding UTF8
$q = "SET NOCOUNT ON; SELECT s.name + '.' + t.name, SUM(p.rows) FROM chartingMinna.sys.tables t JOIN chartingMinna.sys.schemas s ON s.schema_id = t.schema_id " +
     "JOIN chartingMinna.sys.partitions p ON p.object_id = t.object_id AND p.index_id IN (0, 1) GROUP BY s.name, t.name ORDER BY 1;"
$asked = $false
foreach ($inst in 'PRODUCTIONMC\PRODUCTIONMC', '.\PRODUCTIONMC', '.', '10.10.10.3') {
  try {
    $cn = New-Object System.Data.SqlClient.SqlConnection("Server=$inst;Integrated Security=True;Connect Timeout=10;Application Name=charting-capture")
    $cn.Open()
    $cmd = $cn.CreateCommand(); $cmd.CommandText = $q; $cmd.CommandTimeout = 60
    $rd = $cmd.ExecuteReader(); $n = 0
    "-- $inst" | Out-File $out -Append -Encoding UTF8
    while ($rd.Read()) { $n++; ("{0}`t{1}" -f $rd.GetValue(0), $rd.GetValue(1)) | Out-File $out -Append -Encoding UTF8 }
    $cn.Close()
    Log "chartingMinna on ${inst}: $n tables (chartingMinna.txt)" 'Green'; $asked = $true; break
  } catch { Log ("chartingMinna on ${inst}: " + $_.Exception.Message.Split("`n")[0]) 'DarkGray' }
}
if (-not $asked) { Log "chartingMinna could not be read on this server" 'Yellow' }

# Nothing from the documents: list where world files live, so the right folder can be named.
if (-not $found) {
  Log "--- No scans from the documents: looking for world files on D: and E: (top levels only)" 'Cyan'
  foreach ($root in 'D:\', 'E:\') { foreach ($lvl in '', '*\', '*\*\', '*\*\*\') { foreach ($e in '*.tfw', '*.jgw') {
    foreach ($f in @(Get-ChildItem -Path (Join-Path $root ($lvl + $e)) -File | Select-Object -First 200)) { Log "world file $($f.FullName)" }
  } } }
}

Log "--- Paths in the documents that are not on this server (another PC: PLAGIS05, the officers' desktops)" 'Cyan'
foreach ($p in ($missing | Sort-Object)) { Log "not here: $p" 'DarkGray' }
Log ("Finished {0}: {1:N2} GB copied to {2}, {3} files kept from the last run" -f (Get-Date), ($script:copied / 1GB), $cap, $script:reused) 'Green'
