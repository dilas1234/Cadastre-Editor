<#
  READ-ONLY capture of the last pieces of the Cadastre <-> Registry link, for the new Cadastre to
  replace them exactly (Phase 5). Run inside the Remote Desktop session on PRODUCTIONMC
  (RUN-ME-REGISTRY-MAP-CAPTURE.cmd). Output: <OutDir>\lrs-export\map-capture\.

  1. PRODUCTIONMC, the GISViewer the Registry's map window opens (GISServer / LWAMapURL =
     http://10.10.10.6/GISViewer/MapWorker.aspx): its Silverlight client (Client\*.xap and the
     Client folder), every assembly in bin, and its Conf (masked). The Registry's MapWorkerControl
     drives this client through a script bridge; the bridge's names are inside these files.
  2. PRODUCTIONMC, ApplicationServerAdmin (C:\Program Files (x86)\AUMENTUM\CADASTRE\WebAdmin):
     the web services the Registry calls (WebServices\LRSWebServices.asmx and the others), every
     assembly in bin, and the configs (masked). This is Aumentum's code behind the job service
     the new Cadastre now serves, to confirm its behaviour operation by operation.
  3. The Registry server (registry.plagis.com, over remote management as before): the Registry
     desktop client's LRS.Client.* assemblies (LRS.Client.MapWorker holds MapWorkerControl) and
     its configs (masked).

  Copies files only; nothing is changed anywhere. PowerShell 4 compatible.
#>
param([string]$OutDir = $PSScriptRoot, [string]$Target = 'registry.plagis.com', [switch]$SkipRegistry)
$ErrorActionPreference = 'Stop'
$root = Join-Path $OutDir 'lrs-export\map-capture'
New-Item -ItemType Directory -Path $root -Force | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
$report = New-Object Collections.Generic.List[string]
$report.Add("Map / job service capture on $env:COMPUTERNAME, $(Get-Date -Format s)")
function Mask($s) {
  $s -replace '(?i)((password|pwd|passwd|secret|apikey|api_key|token|ticket)\s*[=:]\s*)[^;"''<\s&]+', '$1****' `
     -replace '(?i)(<(password|pwd|passwd|secret|ticket)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|passwd|secret|ticket)"\s*value\s*=\s*")[^"]*', '$1****' `
     -replace '(?i)(name="(password|pwd|passwd|secret)"[^>]*>)[^<]*', '$1****'
}
function ReadShared([string]$path) {
  $share = [IO.FileShare]'ReadWrite, Delete'
  $fs = New-Object IO.FileStream($path, ([IO.FileMode]::Open), ([IO.FileAccess]::Read), $share)
  try { $r = New-Object IO.StreamReader($fs, $true); $r.ReadToEnd() } finally { $fs.Close() }
}
# Copy a folder's files that match: text files masked, binaries as they are.
function CopyTree([string]$src, [string]$dest, [string]$pattern, [string]$label) {
  if (-not (Test-Path $src)) { $report.Add("${label}: not found at $src"); Write-Host "  $label not found ($src)" -ForegroundColor Yellow; return }
  $n = 0
  foreach ($f in (Get-ChildItem $src -Recurse -File -ErrorAction SilentlyContinue | Where { $_.Name -match $pattern -and $_.FullName -notmatch '\\(Logs|LocalStorage|Temp)\\' -and $_.Length -lt 60MB })) {
    $rel = $f.FullName.Substring($src.Length).TrimStart('\')
    $to = Join-Path $dest $rel
    New-Item -ItemType Directory -Path (Split-Path $to -Parent) -Force | Out-Null
    try {
      if ($f.Extension -match '^\.(config|xml|aspx|asmx|ashx|asax|svc|js|htm|html|txt|xaml|wsdl|xsd)$') { [IO.File]::WriteAllText($to, (Mask (ReadShared $f.FullName)), $utf8) }
      else { Copy-Item -LiteralPath $f.FullName -Destination $to }
      $n++
    } catch { $report.Add("${label}: not copied $rel ($($_.Exception.Message))") }
  }
  $report.Add("${label}: $n files from $src")
  Write-Host "  ${label}: $n files" -ForegroundColor DarkGray
}

Write-Host "[1] GISViewer (the Registry's map) ..." -ForegroundColor Cyan
CopyTree 'C:\inetpub\wwwroot\GISViewer\Client' (Join-Path $root 'gisviewer\Client') '\.(xap|xaml|zip|xml|config|js)$' 'GISViewer Client'
CopyTree 'C:\inetpub\wwwroot\GISViewer\bin' (Join-Path $root 'gisviewer\bin') '\.(dll|xml|config)$' 'GISViewer bin'
CopyTree 'C:\inetpub\wwwroot\GISViewer\App_Data\Conf' (Join-Path $root 'gisviewer\Conf') '.' 'GISViewer Conf'
CopyTree 'C:\inetpub\wwwroot\GISViewer' (Join-Path $root 'gisviewer\pages') '^[^\\]+\.(aspx|ashx|asmx|asax|svc|config|htm|html|js)$' 'GISViewer pages'

Write-Host "[2] ApplicationServerAdmin (the Registry's job service) ..." -ForegroundColor Cyan
$wa = @('C:\Program Files (x86)\AUMENTUM\CADASTRE\WebAdmin', 'C:\Program Files\AUMENTUM\CADASTRE\WebAdmin') | Where { Test-Path $_ } | Select -First 1
if ($wa) {
  CopyTree (Join-Path $wa 'WebServices') (Join-Path $root 'webadmin\WebServices') '.' 'WebAdmin WebServices'
  CopyTree (Join-Path $wa 'bin') (Join-Path $root 'webadmin\bin') '\.(dll|xml|config)$' 'WebAdmin bin'
  CopyTree $wa (Join-Path $root 'webadmin\root') '^[^\\]+\.(config|asax|aspx)$' 'WebAdmin root'
} else { $report.Add('WebAdmin not found'); Write-Host '  WebAdmin not found' -ForegroundColor Yellow }

if (-not $SkipRegistry) {
  Write-Host "[3] Registry desktop client (MapWorkerControl), on $Target ..." -ForegroundColor Cyan
  try {
    Test-WSMan -ComputerName $Target | Out-Null
    $s = $null
    try { $s = New-PSSession -ComputerName $Target }
    catch {
      Write-Host "Your sign-in was refused on $Target; enter an administrator account of the Registry server." -ForegroundColor Yellow
      $s = New-PSSession -ComputerName $Target -Credential (Get-Credential)
    }
    try {
      $files = Invoke-Command -Session $s -ScriptBlock {
        $dirs = @('C:\Program Files (x86)\Aumentum Registry', 'E:\Program Files (x86)\Aumentum Registry', 'C:\Program Files\Aumentum Registry') | Where { Test-Path $_ }
        foreach ($d in $dirs) {
          Get-ChildItem $d -Recurse -File -ErrorAction SilentlyContinue |
            Where { ($_.Name -match '^(LRS\.Client\.|LRS\.Core|ILS\.MCPortal|.*MapWorker)' -and $_.Extension -eq '.dll') -or ($_.Extension -eq '.config' -and $_.Length -lt 2MB) } |
            ForEach-Object { New-Object PSObject -Property @{ Full = $_.FullName; Rel = $_.FullName.Substring($d.Length).TrimStart('\'); Len = $_.Length } }
        }
      }
      $n = 0
      foreach ($f in $files) {
        if ($f.Len -gt 8MB) { $report.Add("Registry client: skipped (large) $($f.Rel)"); continue }
        $b64 = Invoke-Command -Session $s -ArgumentList $f.Full -ScriptBlock { param($p) [Convert]::ToBase64String([IO.File]::ReadAllBytes($p)) }
        $to = Join-Path $root ('registry-client\' + $f.Rel)
        New-Item -ItemType Directory -Path (Split-Path $to -Parent) -Force | Out-Null
        $bytes = [Convert]::FromBase64String($b64)
        if ($f.Rel -match '\.config$') { [IO.File]::WriteAllText($to, (Mask ($utf8.GetString($bytes))), $utf8) } else { [IO.File]::WriteAllBytes($to, $bytes) }
        $n++
      }
      $report.Add("Registry client: $n files from $Target")
      Write-Host "  Registry client: $n files" -ForegroundColor DarkGray
    } finally { if ($s) { Remove-PSSession $s } }
  } catch {
    $report.Add("Registry client: stopped ($($_.Exception.Message))")
    Write-Host "  Registry part stopped: $($_.Exception.Message)" -ForegroundColor Red
  }
}
[IO.File]::WriteAllLines((Join-Path $root 'map-capture-summary.txt'), [string[]]$report, $utf8)
Write-Host "`nDone. Results are in $root" -ForegroundColor Green
