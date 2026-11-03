# Starts the Cadastre server task and waits for GET /health. Run elevated.
param([string]$TaskName = 'Cadastre Server')
$ErrorActionPreference = 'Stop'
$dir = Split-Path -Parent $MyInvocation.MyCommand.Path

Start-ScheduledTask -TaskName $TaskName

$bind = (Get-Content (Join-Path $dir 'cadastre.env') | Where-Object { $_ -match '^\s*CADASTRE_BIND\s*=' } | Select-Object -First 1)
$port = if ($bind -match ':(\d+)\s*$') { $Matches[1] } else { '8080' }
$url = "http://localhost:$port/health"

for ($i = 0; $i -lt 60; $i++) {
  Start-Sleep -Seconds 2
  try {
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 3
    if ($r.StatusCode -eq 200) { Write-Host "Cadastre server is up: http://${env:COMPUTERNAME}:$port/editor"; exit 0 }
  } catch { }
}
Write-Warning "No answer from $url after 2 minutes. Last log lines:"
Get-Content (Join-Path $dir 'logs\cadastre-server.log') -Tail 30 -ErrorAction SilentlyContinue
exit 1
