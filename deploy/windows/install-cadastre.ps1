<#
  Installs or upgrades the Cadastre server on Windows Server 2012 R2 (PowerShell 4).
  Run from the unzipped kit folder, in an elevated PowerShell:

    powershell -ExecutionPolicy Bypass -File .\install-cadastre.ps1

  - Copies cadastre-server.exe and the scripts to $InstallDir (default C:\Cadastre).
    An existing cadastre.env, logs\ and files\ are kept, so this is also the upgrade path.
  - Registers the "Cadastre Server" scheduled task: runs at boot as SYSTEM, no time limit.
  - Opens the HTTP port from CADASTRE_BIND in Windows Firewall.
  - Starts the server unless cadastre.env still needs editing.
#>
param(
  [string]$InstallDir = 'C:\Cadastre',
  [string]$TaskName = 'Cadastre Server'
)
$ErrorActionPreference = 'Stop'
$kit = Split-Path -Parent $MyInvocation.MyCommand.Path

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'Run this from an elevated (Run as administrator) PowerShell.'
}

# Stop a running instance before replacing the exe.
& (Join-Path $kit 'stop-cadastre.ps1') -TaskName $TaskName

New-Item -ItemType Directory -Force -Path $InstallDir, "$InstallDir\logs", "$InstallDir\files" | Out-Null
foreach ($f in 'cadastre-server.exe', 'run-cadastre.cmd', 'start-cadastre.ps1', 'stop-cadastre.ps1',
               'install-cadastre.ps1', 'setup-database.ps1', 'cadastre.env.example', 'VERSION.txt', 'README.md') {
  $src = Join-Path $kit $f
  if ((Test-Path $src) -and ((Resolve-Path $src).Path -ne (Join-Path $InstallDir $f))) {
    Copy-Item $src $InstallDir -Force
  }
}

$envFile = Join-Path $InstallDir 'cadastre.env'
$fresh = -not (Test-Path $envFile)
if ($fresh) { Copy-Item (Join-Path $InstallDir 'cadastre.env.example') $envFile }
# cadastre.env holds passwords: Administrators and SYSTEM only.
& icacls $envFile /inheritance:r /grant:r 'Administrators:F' 'SYSTEM:F' | Out-Null

# Scheduled task: at startup, as SYSTEM, never time out (the default limit is 3 days).
$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument "/c `"$InstallDir\run-cadastre.cmd`"" -WorkingDirectory $InstallDir
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew
$taskPrincipal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings `
  -Principal $taskPrincipal -Force | Out-Null
Write-Host "Registered scheduled task '$TaskName'."

# Firewall rule for the bind port.
$bind = (Get-Content $envFile | Where-Object { $_ -match '^\s*CADASTRE_BIND\s*=' } | Select-Object -First 1)
$port = if ($bind -match ':(\d+)\s*$') { [int]$Matches[1] } else { 8080 }
$ruleName = "Cadastre Server (TCP $port)"
Get-NetFirewallRule -DisplayName 'Cadastre Server (TCP *' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Protocol TCP -LocalPort $port -Action Allow | Out-Null
Write-Host "Firewall: inbound TCP $port allowed."

if ($fresh -or (Select-String -Path $envFile -Pattern 'CHANGE_ME' -Quiet)) {
  Write-Warning "Edit $envFile (database URL, port), then run: $InstallDir\start-cadastre.ps1"
} else {
  & (Join-Path $InstallDir 'start-cadastre.ps1') -TaskName $TaskName
}
