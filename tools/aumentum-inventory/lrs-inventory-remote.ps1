<#
  Runs the READ-ONLY Registry inventory (lrs-inventory.ps1) on registry.plagis.com (10.10.10.5)
  from PRODUCTIONMC over Windows remote management (WinRM, port 5985), so no Remote Desktop to
  the Registry is needed. Run inside the Remote Desktop session on PRODUCTIONMC.

  What it does on the Registry server:
    - runs lrs-inventory.ps1 there (Get-* commands, file reads, ?wsdl GETs, SELECT statements only)
    - its report goes to a new folder under that user's TEMP, is copied back here, and that temp
      folder (created by this run) is then removed. Nothing else on the Registry server changes.
  Uses your current Windows sign-in; if that is refused, it asks for an account that is an
  administrator on the Registry server. PowerShell 4 compatible.
#>
param([string]$OutDir = $PSScriptRoot, [string]$Target = 'registry.plagis.com', [int]$From = 1)
# Optional SQL login for the Registry database, from lrs-sql.env next to this file (git-ignored):
#   LRS_SQL_USER=...   LRS_SQL_PASSWORD=...
# It is passed to the Registry server inside the encrypted remote session and used only in memory.
$sqlUser = ''; $sqlPassword = ''
$envFile = Join-Path $PSScriptRoot 'lrs-sql.env'
if (Test-Path $envFile) {
  foreach ($line in (Get-Content $envFile)) {
    if ($line -match '^\s*LRS_SQL_USER\s*=\s*(.*?)\s*$') { $sqlUser = $Matches[1] }
    if ($line -match '^\s*LRS_SQL_PASSWORD\s*=\s*(.*?)\s*$') { $sqlPassword = $Matches[1] }
  }
  Write-Host "Using SQL login '$sqlUser' for the Registry database (from lrs-sql.env)." -ForegroundColor DarkGray
}
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'lrs-inventory.ps1'
if (-not (Test-Path $script)) { Write-Host "lrs-inventory.ps1 must be next to this file." -ForegroundColor Red; exit 1 }

Write-Host "Checking remote management on $Target ..." -ForegroundColor Cyan
try { Test-WSMan -ComputerName $Target | Out-Null } catch { Write-Host "WinRM on $Target is not reachable: $($_.Exception.Message)" -ForegroundColor Red; exit 1 }

$s = $null
try { $s = New-PSSession -ComputerName $Target }
catch {
  Write-Host "Your current sign-in was refused on $Target ($($_.Exception.Message))." -ForegroundColor Yellow
  Write-Host "Enter an account that is an administrator on the Registry server (e.g. PLAGIS\Administrator)." -ForegroundColor Yellow
  $cred = Get-Credential
  $s = New-PSSession -ComputerName $Target -Credential $cred
}

try {
  $remoteDir = Invoke-Command -Session $s -ScriptBlock {
    $d = Join-Path $env:TEMP ('lrs-inventory-' + (Get-Date -Format yyyyMMdd-HHmmss))
    New-Item -ItemType Directory -Path $d -Force | Out-Null; $d
  }
  Write-Host "Running the read-only Registry inventory on $Target (this can take 10-20 minutes) ..." -ForegroundColor Cyan
  Invoke-Command -Session $s -FilePath $script -ArgumentList $remoteDir, '', 'http://localhost:8080/lrs/api/', $sqlUser, $sqlPassword, $From

  Write-Host "Copying the results back to $OutDir ..." -ForegroundColor Cyan
  $files = Invoke-Command -Session $s -ArgumentList $remoteDir -ScriptBlock {
    param($d)
    # TEMP may be reported in 8.3 short form (ADMINI~1) while file paths are long, so cut each
    # path at this run's own folder name, which is never shortened in the long form.
    $leaf = '\' + (Split-Path $d -Leaf) + '\'
    Get-ChildItem $d -Recurse -File | ForEach-Object {
      $i = $_.FullName.IndexOf($leaf)
      $rel = if ($i -ge 0) { $_.FullName.Substring($i + $leaf.Length) } else { $_.Name }
      New-Object PSObject -Property @{ Rel = $rel; B64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($_.FullName)) }
    }
  }
  foreach ($f in $files) {
    $dest = Join-Path $OutDir $f.Rel
    New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
    [IO.File]::WriteAllBytes($dest, [Convert]::FromBase64String($f.B64))
    Write-Host "  $($f.Rel)"
  }
  # Remove only the temp folder this run created on the Registry server.
  Invoke-Command -Session $s -ArgumentList $remoteDir -ScriptBlock { param($d) if ($d -like '*\lrs-inventory-*') { Remove-Item $d -Recurse -Force } }
  Write-Host "`nDone. $(@($files).Count) files copied to $OutDir" -ForegroundColor Green
}
finally { if ($s) { Remove-PSSession $s } }
