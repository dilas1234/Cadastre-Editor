<#
  Phase 5 capture for the Cadastre <-> Registry linkage. Run inside the Remote Desktop session on
  PRODUCTIONMC (RUN-ME-REGISTRY-EXPORT.cmd). READ-ONLY on both machines.

  1. Registry (registry.plagis.com, 10.10.10.5), over remote management (WinRM) as for the
     inventory: runs lrs-export.ps1 there (structure, linkage rows without personal data, LRS
     model definitions), copies the result back in chunks, then removes only the temp folder this
     run created there.
  2. PRODUCTIONMC itself, the Cadastre side of the link:
       - the AUMENTUM REGISTRY extension of the Cadastre desktop (Grm.Cadastre.RegistryExtension
         and the LRS client libraries it calls: IntegrationService, ServiceProxy, WorkFlowService,
         LRS.*, ILS.*), its config (masked) and logs. They hold the link/unlink rules, the
         send-image flow and the SOAP client types (lrs-service-types).
       - the COM registrations of the desktop's commands (CLSID -> class), so every step-tool
         GUID in job.xml can be named, the Registry ones included.
       - the GISViewer web app (C:\inetpub\wwwroot\GISViewer) that the Registry calls back
         (GISServer / LWAMapURL = http://10.10.10.6/GISViewer/MapWorker.aspx): page and handler
         files, its configuration (masked), its bin file list with the LRS/MapWorker assemblies,
         and the last 60 days of its request log.
  Output: <OutDir>\lrs-export\ (back on the Linux machine via \\tsclient\inventory).
  PowerShell 4 compatible.
#>
# -Resume: do not export again; copy back what an earlier run left on the Registry server (its
# newest lrs-export-* temp folder, or -RemoteDir), skipping files already copied complete.
param([string]$OutDir = $PSScriptRoot, [string]$Target = 'registry.plagis.com', [switch]$SkipRegistry, [switch]$SkipLocal,
      [switch]$Resume, [string]$RemoteDir = '')
$ErrorActionPreference = 'Stop'
$root = Join-Path $OutDir 'lrs-export'
New-Item -ItemType Directory -Path $root -Force | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
function Mask($s) {
  $s -replace '(?i)((password|pwd|passwd|secret|apikey|api_key|token|ticket)\s*[=:]\s*)[^;"''<\s&]+', '$1****' `
     -replace '(?i)(<(password|pwd|passwd|secret|ticket)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|passwd|secret|ticket)"\s*value\s*=\s*")[^"]*', '$1****' `
     -replace '(?i)(name="(password|pwd|passwd|secret)"[^>]*>)[^<]*', '$1****'
}

# ------------------------------------------------------------------ 1. the Registry
if (-not $SkipRegistry) {
  $sqlUser = ''; $sqlPassword = ''
  $envFile = Join-Path $PSScriptRoot 'lrs-sql.env'
  if (Test-Path $envFile) {
    foreach ($line in (Get-Content $envFile)) {
      if ($line -match '^\s*LRS_SQL_USER\s*=\s*(.*?)\s*$') { $sqlUser = $Matches[1] }
      if ($line -match '^\s*LRS_SQL_PASSWORD\s*=\s*(.*?)\s*$') { $sqlPassword = $Matches[1] }
    }
    Write-Host "Using SQL login '$sqlUser' for the Registry database (from lrs-sql.env)." -ForegroundColor DarkGray
  }
  $script = Join-Path $PSScriptRoot 'lrs-export.ps1'
  if (-not (Test-Path $script)) { Write-Host "lrs-export.ps1 must be next to this file." -ForegroundColor Red; exit 1 }
  Write-Host "Checking remote management on $Target ..." -ForegroundColor Cyan
  try { Test-WSMan -ComputerName $Target | Out-Null } catch { Write-Host "WinRM on $Target is not reachable: $($_.Exception.Message)" -ForegroundColor Red; exit 1 }
  $s = $null
  try { $s = New-PSSession -ComputerName $Target }
  catch {
    Write-Host "Your current sign-in was refused on $Target ($($_.Exception.Message))." -ForegroundColor Yellow
    Write-Host "Enter an account that is an administrator on the Registry server (e.g. PLAGIS\Administrator)." -ForegroundColor Yellow
    $s = New-PSSession -ComputerName $Target -Credential (Get-Credential)
  }
  try {
    if ($Resume) {
      $remoteDir = Invoke-Command -Session $s -ArgumentList $RemoteDir -ScriptBlock {
        param($d)
        if ($d) { if (Test-Path $d) { return $d } else { return $null } }
        Get-ChildItem $env:TEMP -Directory -Filter 'lrs-export-*' | Where { Test-Path (Join-Path $_.FullName 'lrs-export-summary.txt') } |
          Sort LastWriteTime -Descending | Select -First 1 -Expand FullName
      }
      if (-not $remoteDir) { Write-Host "No finished export found on $Target to resume from; run without -Resume." -ForegroundColor Red; exit 1 }
      Write-Host "Resuming the copy of $remoteDir from $Target (no new export) ..." -ForegroundColor Cyan
    } else {
      $remoteDir = Invoke-Command -Session $s -ScriptBlock { Join-Path $env:TEMP ('lrs-export-' + (Get-Date -Format yyyyMMdd-HHmmss)) }
      Write-Host "Running the read-only Registry export on $Target (about 1-2 hours) ..." -ForegroundColor Cyan
      Invoke-Command -Session $s -FilePath $script -ArgumentList $remoteDir, '', $sqlUser, $sqlPassword
    }
    $files = Invoke-Command -Session $s -ArgumentList $remoteDir -ScriptBlock {
      param($d)
      $leaf = '\' + (Split-Path $d -Leaf) + '\'
      Get-ChildItem $d -Recurse -File | ForEach-Object {
        $i = $_.FullName.IndexOf($leaf)
        New-Object PSObject -Property @{ Full = $_.FullName; Rel = $(if ($i -ge 0) { $_.FullName.Substring($i + $leaf.Length) } else { $_.Name }); Len = $_.Length }
      }
    }
    # The structure and rows first, the bulky application files (jars) last.
    $files = @($files | Sort-Object @{ e = { if ($_.Rel -like 'app\*') { 1 } else { 0 } } }, Rel)
    $total = ($files | Measure-Object Len -Sum).Sum
    Write-Host ("Copying {0} files ({1:N1} MB) back to {2} ..." -f @($files).Count, ($total / 1MB), $root) -ForegroundColor Cyan
    # 4 MB per call: PowerShell 3/4 remoting refuses any object over 10 MB (an 8 MB chunk is
    # 10.7 MB as base64, which is what stopped the first run at the first jar).
    $chunk = 4MB; $done = 0; $skipped = 0
    foreach ($f in $files) {
      $dest = Join-Path $root ('registry\' + $f.Rel)
      $done += $f.Len
      if ((Test-Path $dest) -and (Get-Item $dest).Length -eq $f.Len) { $skipped++; continue }
      New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
      $out = [IO.File]::Create($dest)
      try {
        for ($off = 0; $off -lt $f.Len; $off += $chunk) {
          $b64 = Invoke-Command -Session $s -ArgumentList $f.Full, $off, $chunk -ScriptBlock {
            param($p, $o, $n)
            $fs = [IO.File]::OpenRead($p)
            try { [void]$fs.Seek($o, 'Begin'); $buf = New-Object byte[] ([Math]::Min($n, $fs.Length - $o)); $read = $fs.Read($buf, 0, $buf.Length); [Convert]::ToBase64String($buf, 0, $read) }
            finally { $fs.Close() }
          }
          $bytes = [Convert]::FromBase64String($b64); $out.Write($bytes, 0, $bytes.Length)
        }
      } finally { $out.Close() }
      if ((Get-Item $dest).Length -ne $f.Len) { throw "copy of $($f.Rel) is incomplete" }
      Write-Host ("  {0,5:N1}%  {1}" -f (100.0 * $done / [Math]::Max(1, $total)), $f.Rel)
    }
    if ($skipped) { Write-Host "  ($skipped files were already here complete)" -ForegroundColor DarkGray }
    # Only after every file is here complete: remove the temp folder this export created there.
    Invoke-Command -Session $s -ArgumentList $remoteDir -ScriptBlock { param($d) if ($d -like '*\lrs-export-*') { Remove-Item $d -Recurse -Force } }
    Write-Host "Registry part done." -ForegroundColor Green
  }
  finally { if ($s) { Remove-PSSession $s } }
}

# ------------------------------------------------------------------ 2. PRODUCTIONMC (Cadastre side)
if (-not $SkipLocal) {
  $mc = Join-Path $root 'productionmc'
  New-Item -ItemType Directory -Path $mc -Force | Out-Null
  $report = New-Object Collections.Generic.List[string]
  $report.Add("Cadastre side of the Registry link on $env:COMPUTERNAME, $(Get-Date -Format s)")

  Write-Host "[a] AUMENTUM REGISTRY extension and the LRS client libraries ..." -ForegroundColor Cyan
  $desk = @('C:\Program Files (x86)\AUMENTUM\CADASTRE\Desktop', 'C:\Program Files\AUMENTUM\CADASTRE\Desktop') | Where { Test-Path $_ } | Select -First 1
  if ($desk) {
    $ext = Join-Path $mc 'desktop'; New-Item -ItemType Directory -Path $ext -Force | Out-Null
    $libs = '^(Grm\.Cadastre\.RegistryExtension|IntegrationService|ServiceProxy|WorkFlowService|AuditService|LRS\.|ILS\.|ServiceAgent\.|Aumentum\.Cadastre\.Remote|Ils\.MultiCadastre\.Desktop|Ils\.MultiCadastre\.Extension|Ils\.MultiCadastre\.ParcelLineAge)'
    foreach ($f in (Get-ChildItem $desk -File | Where { $_.Name -match $libs -and $_.Extension -match '^\.(dll|tlb|exe)$' })) { Copy-Item $f.FullName $ext; $report.Add("copied $($f.Name) ($($f.Length) bytes, $($f.VersionInfo.FileVersion))") }
    foreach ($f in (Get-ChildItem $desk -File | Where { $_.Extension -match '^\.(config|xml|settings)$' })) {
      [IO.File]::WriteAllText((Join-Path $ext $f.Name), (Mask ([IO.File]::ReadAllText($f.FullName))), $utf8); $report.Add("copied (masked) $($f.Name)")
    }
  } else { $report.Add('Cadastre Desktop folder not found') }

  Write-Host "[b] COM registrations of the desktop's commands (CLSID -> class) ..." -ForegroundColor Cyan
  $rows = New-Object Collections.Generic.List[string]; $rows.Add('clsid|class|assembly|codebase|hive')
  foreach ($hive in 'Registry::HKEY_CLASSES_ROOT\CLSID', 'Registry::HKEY_CLASSES_ROOT\Wow6432Node\CLSID') {
    if (-not (Test-Path $hive)) { continue }
    foreach ($k in (Get-ChildItem $hive -ErrorAction SilentlyContinue)) {
      $ip = Join-Path $k.PSPath 'InprocServer32'
      if (-not (Test-Path $ip)) { continue }
      $p = Get-ItemProperty $ip -ErrorAction SilentlyContinue
      if ($p -and ($p.Assembly -match 'Grm\.|Ils\.|Aumentum|LRS|MultiCadastre' -or $p.Class -match 'Grm\.|Ils\.|Aumentum|LRS')) {
        $rows.Add("$($k.PSChildName)|$($p.Class)|$($p.Assembly)|$($p.CodeBase)|$(Split-Path $hive -Leaf)")
      }
    }
  }
  [IO.File]::WriteAllLines((Join-Path $mc 'com-commands.psv'), [string[]]$rows, $utf8)
  $report.Add("COM commands registered: $($rows.Count - 1)")

  Write-Host "[c] GISViewer (called back by the Registry) ..." -ForegroundColor Cyan
  $gv = 'C:\inetpub\wwwroot\GISViewer'
  if (Test-Path $gv) {
    $gvOut = Join-Path $mc 'gisviewer'
    $since = (Get-Date).AddDays(-60)
    $files = Get-ChildItem $gv -Recurse -File -ErrorAction SilentlyContinue
    $list = New-Object Collections.Generic.List[string]; $list.Add('path|bytes|modified|version')
    foreach ($f in $files) {
      $rel = $f.FullName.Substring($gv.Length).TrimStart('\')
      $list.Add("$rel|$($f.Length)|$($f.LastWriteTime.ToString('s'))|$(if ($f.Extension -eq '.dll') { $f.VersionInfo.FileVersion })")
      $text = $f.Extension -match '^\.(aspx|ashx|asmx|asax|config|xml|js|json|htm|html|txt|svc|xsd|wsdl)$' -and $f.Length -lt 5MB -and $rel -notmatch '^App_Data\\Logs\\'
      $log = $rel -match '^App_Data\\Logs\\' -and $f.LastWriteTime -ge $since
      $asm = $f.Extension -eq '.dll' -and $f.Name -match '(?i)LRS|MapWorker|ILS\.|Grm\.|Aumentum|MCPortal'
      if ($text -or $log -or $asm) {
        $dest = Join-Path $gvOut $rel; New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
        if ($text -or $log) { [IO.File]::WriteAllText($dest, (Mask ([IO.File]::ReadAllText($f.FullName))), $utf8) } else { Copy-Item $f.FullName $dest }
      }
    }
    New-Item -ItemType Directory -Path $gvOut -Force | Out-Null
    [IO.File]::WriteAllLines((Join-Path $gvOut '_files.psv'), [string[]]$list, $utf8)
    $report.Add("GISViewer: $($files.Count) files listed")
  } else { $report.Add('GISViewer folder not found') }
  [IO.File]::WriteAllLines((Join-Path $mc 'productionmc-summary.txt'), [string[]]$report, $utf8)
  Write-Host "PRODUCTIONMC part done." -ForegroundColor Green
}
Write-Host "`nAll done. Results are in $root" -ForegroundColor Green
