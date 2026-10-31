<#
  READ-ONLY inventory of the Aumentum Cadastre Editor host.
  - Uses only Get-*/query commands. Changes no settings, services, registry or files.
  - Its only write is the report file (to -OutDir; default your Desktop).
  - Masks password/secret values found in config files.
  - Each section is appended to the report as soon as it finishes, so partial results survive
    and you can watch progress in the window. Folder scans are depth- and count-limited.
  - PowerShell 4 compatible (Windows Server 2012 R2).
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'))
$ErrorActionPreference = 'SilentlyContinue'
$out = Join-Path $OutDir ("aumentum-inventory-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format yyyyMMdd-HHmm))
"Aumentum inventory of $env:COMPUTERNAME started $(Get-Date)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $out" -ForegroundColor Red; exit 1 }

$script:n = 0
function Section([string]$title, [scriptblock]$body) {
  $script:n++
  $t0 = Get-Date
  Write-Host ("[{0:HH:mm:ss}] {1,2}/17  {2} ..." -f $t0, $script:n, $title) -ForegroundColor Cyan
  $text = try { & $body *>&1 | Out-String -Width 300 } catch { "ERROR: $_" }
  "`r`n==================== $title ====================`r`n$text" | Out-File $out -Append -Encoding UTF8
  Write-Host ("            done in {0:N0}s" -f ((Get-Date) - $t0).TotalSeconds) -ForegroundColor DarkGray
}
function Mask($s) {
  $s -replace '(?i)((password|pwd|secret|apikey|api_key|token)\s*[=:]\s*)[^;"''<\s]+', '$1****' `
     -replace '(?i)(<(password|pwd|secret)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|secret)"\s*value\s*=\s*")[^"]*', '$1****'
}
# Depth-limited walk that never descends past $max levels and stops after $limit items.
function Walk([string]$path, [int]$max = 3, [int]$limit = 2000) {
  $queue = New-Object System.Collections.Queue
  $queue.Enqueue(@($path, 0)); $count = 0
  while ($queue.Count -gt 0 -and $count -lt $limit) {
    $p, $d = $queue.Dequeue()
    foreach ($i in (Get-ChildItem -LiteralPath $p -Force)) {
      $count++; $i
      if ($i.PSIsContainer -and $d + 1 -lt $max) { $queue.Enqueue(@($i.FullName, ($d + 1))) }
      if ($count -ge $limit) { break }
    }
  }
}
$pat = 'Aumentum|Cadastr|Esri|ArcGIS|ArcSDE|Plateau|PLAGIS|Thomson|Manatron|LRS'

Section "HOST / OS" {
  Get-WmiObject Win32_OperatingSystem | Select Caption, Version, BuildNumber, OSArchitecture, LastBootUpTime | Format-List
  Get-WmiObject Win32_ComputerSystem | Select Name, Domain, Manufacturer, Model, NumberOfLogicalProcessors, @{n='RAM_GB';e={[math]::Round($_.TotalPhysicalMemory/1GB,1)}} | Format-List
  Get-WmiObject Win32_LogicalDisk -Filter "DriveType=3" | Select DeviceID, @{n='SizeGB';e={[math]::Round($_.Size/1GB)}}, @{n='FreeGB';e={[math]::Round($_.FreeSpace/1GB)}} | Format-Table -Auto
  Get-WmiObject Win32_NetworkAdapterConfiguration -Filter "IPEnabled=True" | Select Description, IPAddress, DefaultIPGateway, DNSServerSearchOrder | Format-List
  "PowerShell: $($PSVersionTable.PSVersion)"
  ".NET 4 release key: " + (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\NET Framework Setup\NDP\v4\Full').Release
}
Section "INSTALLED SOFTWARE" {
  Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' |
    Where DisplayName | Select DisplayName, DisplayVersion, Publisher, InstallDate, InstallLocation | Sort DisplayName | Format-Table -Auto -Wrap
}
Section "WINDOWS FEATURES / ROLES" { Get-WindowsFeature | Where Installed | Select Name, DisplayName | Format-Table -Auto }
Section "SERVICES" { Get-WmiObject Win32_Service | Select Name, State, StartMode, StartName, PathName | Sort Name | Format-Table -Auto -Wrap }
Section "PROCESSES" { Get-WmiObject Win32_Process | Select ProcessId, Name, ExecutablePath, CommandLine | Sort Name | Format-Table -Auto -Wrap }
Section "LISTENING PORTS / CONNECTIONS" { netstat -ano }
Section "SCHEDULED TASKS (non-Microsoft)" {
  schtasks /query /fo CSV /v | ConvertFrom-Csv | Where { $_.TaskName -notlike '\Microsoft\*' -and $_.TaskName -ne 'TaskName' } |
    Select TaskName, Status, 'Run As User', 'Task To Run', 'Schedule Type', 'Last Run Time' | Format-List
}
Section "IIS SITES / APP POOLS / APPLICATIONS" {
  if (Get-Module -ListAvailable WebAdministration) {
    Import-Module WebAdministration
    Get-Website | Select Name, State, PhysicalPath, @{n='Bindings';e={($_.bindings.Collection | % { "$($_.protocol) $($_.bindingInformation)" }) -join '; '}} | Format-Table -Auto -Wrap
    Get-ChildItem IIS:\AppPools | Select Name, State, managedRuntimeVersion, managedPipelineMode, @{n='Identity';e={$_.processModel.identityType}} | Format-Table -Auto
    Get-WebApplication | Select Path, PhysicalPath, applicationPool | Format-Table -Auto -Wrap
  } else { "IIS not installed / WebAdministration unavailable" }
}
Section "ODBC DSNs / SQL CLIENT ALIASES" {
  Get-ItemProperty 'HKLM:\SOFTWARE\ODBC\ODBC.INI\*','HKLM:\SOFTWARE\Wow6432Node\ODBC\ODBC.INI\*' | Select PSChildName, Server, Database, Driver | Format-Table -Auto
  Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\MSSQLServer\Client\ConnectTo','HKLM:\SOFTWARE\Wow6432Node\Microsoft\MSSQLServer\Client\ConnectTo' | Format-List
}
Section "VENDOR REGISTRY (Esri / Aumentum / Thomson Reuters / ...), 2 levels" {
  foreach ($v in 'ESRI','Aumentum','Thomson Reuters','Manatron','Harris','Plateau','PLAGIS') {
    foreach ($root in "HKLM:\SOFTWARE\$v","HKLM:\SOFTWARE\Wow6432Node\$v") {
      if (Test-Path $root) {
        foreach ($k in @(Get-Item $root) + @(Get-ChildItem $root) + @(Get-ChildItem $root | Get-ChildItem)) {
          "[$($k.Name)]"; Mask (Get-ItemProperty $k.PSPath | Select * -Exclude PS* | Format-List | Out-String)
        }
      }
    }
  }
}
$dirs = foreach ($r in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, 'C:\inetpub', 'C:\', 'D:\', 'E:\')) {
  if ($r -and (Test-Path $r)) { Get-ChildItem $r | Where { $_.PSIsContainer -and $_.Name -match $pat } }
}
Section "CANDIDATE INSTALL FOLDERS (3 levels, max 2000 entries each)" {
  foreach ($d in $dirs) {
    "### $($d.FullName)"
    Walk $d.FullName 3 2000 | Select @{n='Path';e={$_.FullName.Substring($d.FullName.Length)}}, Length, LastWriteTime | Format-Table -Auto -Wrap
  }
}
Section "EXE/DLL VERSIONS (4 levels, max 400 per folder)" {
  foreach ($d in $dirs) {
    "### $($d.FullName)"
    Walk $d.FullName 4 20000 | Where { $_.Extension -match '^\.(exe|dll)$' } | Select -First 400 |
      % { [pscustomobject]@{File=$_.FullName.Substring($d.FullName.Length); Version=$_.VersionInfo.FileVersion; Product=$_.VersionInfo.ProductName; Company=$_.VersionInfo.CompanyName} } | Format-Table -Auto -Wrap
  }
}
Section "CONFIG FILES (secrets masked; 4 levels, max 60 per folder)" {
  foreach ($d in $dirs) {
    Walk $d.FullName 4 20000 | Where { $_.Extension -match '^\.(config|ini|json|sde|udl|properties)$' -and $_.Length -lt 200KB } | Select -First 60 |
      % { "----- $($_.FullName) -----"; Mask ([IO.File]::ReadAllText($_.FullName)) }
  }
}
Section "CLICKONCE / USER APPS" {
  Walk "$env:LOCALAPPDATA\Apps\2.0" 6 5000 | Where { $_.Extension -eq '.exe' } | Select FullName, @{n='Version';e={$_.VersionInfo.FileVersion}} | Format-Table -Auto -Wrap
}
Section "SHARES" { Get-WmiObject Win32_Share | Select Name, Path, Description | Format-Table -Auto }
Section "APP EVENT LOG (30 days, Aumentum/Esri/SQL/IIS, max 200)" {
  Get-EventLog -LogName Application -After (Get-Date).AddDays(-30) -Newest 3000 |
    Where { $_.Source -match "$pat|MSSQL|ASP.NET|IIS" -or $_.Message -match 'Aumentum|ArcGIS|ArcSDE' } |
    Select -First 200 TimeGenerated, EntryType, Source, @{n='Message';e={ ($_.Message -split "`n")[0..4] -join ' ' }} | Format-Table -Auto -Wrap
}
Section "END" { "Finished $(Get-Date)" }
Write-Host "`nDone. Report written to: $out" -ForegroundColor Green
