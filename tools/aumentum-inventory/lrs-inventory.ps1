<#
  READ-ONLY inventory of the Aumentum Registry (LRS43) host, 10.10.10.5.
  Captures what the new Cadastre needs to reproduce the Cadastre <-> Registry linkage exactly:
    - host, services, the Java/JBoss process and who connects to it (10.10.10.6 = Cadastre)
    - the LRS application's configuration (datasources, web.xml, Axis server-config.wsdd), masked
    - the SOAP service definitions (WSDL), fetched from this machine's own :8080 and saved
    - the LRS database: tables, row counts, columns, foreign keys, code/lookup tables, settings
      tables (the 13 Cadastre integration keys live here), and link columns (UPIN, PIN, transaction,
      status) as counts and ranges only
  Changes nothing: Get-* commands, file reads, HTTP GETs of ?wsdl, and SELECT statements
  (READ UNCOMMITTED). Writes only to -OutDir. Personal data (owner names, addresses, payment
  details) is not selected: samples come only from small code/settings tables.
  PowerShell 4 compatible.
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'), [string]$SqlInstance = '', [string]$LrsUrl = 'http://localhost:8080/lrs/api/',
      [string]$SqlUser = '', [string]$SqlPassword = '', [int]$From = 1)
$ErrorActionPreference = 'SilentlyContinue'
$out = Join-Path $OutDir ("lrs-inventory-{0}-{1}.txt" -f $env:COMPUTERNAME, (Get-Date -Format yyyyMMdd-HHmm))
"LRS inventory of $env:COMPUTERNAME started $(Get-Date)" | Out-File $out -Encoding UTF8
if (-not (Test-Path $out)) { Write-Host "Cannot write to $out" -ForegroundColor Red; exit 1 }
$cap = Join-Path $OutDir 'lrs-capture'
New-Item -ItemType Directory -Path (Join-Path $cap 'wsdl') -Force | Out-Null

$script:n = 0; $total = 13
function Section([string]$title, [scriptblock]$body) {
  $script:n++; $t0 = Get-Date
  if ($script:n -lt $From) { Write-Host ("            {0,2}/{1}  {2}: skipped (-From {3})" -f $script:n, $total, $title, $From) -ForegroundColor DarkGray; return }
  Write-Host ("[{0:HH:mm:ss}] {1,2}/{2}  {3} ..." -f $t0, $script:n, $total, $title) -ForegroundColor Cyan
  $text = try { & $body *>&1 | Out-String -Width 400 } catch { "ERROR: $_" }
  "`r`n==================== $title ====================`r`n$text" | Out-File $out -Append -Encoding UTF8
  Write-Host ("            done in {0:N0}s" -f ((Get-Date) - $t0).TotalSeconds) -ForegroundColor DarkGray
}
function Mask($s) {
  $s -replace '(?i)((password|pwd|passwd|secret|apikey|api_key|token|ticket)\s*[=:]\s*)[^;"''<\s&]+', '$1****' `
     -replace '(?i)(<(password|pwd|passwd|secret|ticket)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|passwd|secret|ticket)"\s*value\s*=\s*")[^"]*', '$1****' `
     -replace '(?i)(name="(password|pwd|passwd|secret)"[^>]*>)[^<]*', '$1****'
}
function Shallow([string[]]$roots, [string[]]$names, [switch]$Dirs) {
  foreach ($r in $roots) { foreach ($lvl in '', '*\', '*\*\', '*\*\*\') { foreach ($nm in $names) {
    if ($Dirs) { Get-ChildItem -Path (Join-Path $r ($lvl + $nm)) -Directory } else { Get-ChildItem -Path (Join-Path $r ($lvl + $nm)) -File }
  } } }
}
$drives = Get-PSDrive -PSProvider FileSystem | Where { $_.Used -gt 0 } | % { $_.Root }

Section "HOST" {
  Get-WmiObject Win32_OperatingSystem | Select Caption, Version, OSArchitecture, LastBootUpTime, InstallDate | Format-List
  Get-WmiObject Win32_ComputerSystem | Select Name, Domain, Manufacturer, Model, NumberOfLogicalProcessors, @{n='RAM_GB';e={[math]::Round($_.TotalPhysicalMemory/1GB)}} | Format-List
  Get-PSDrive -PSProvider FileSystem | Select Name, @{n='UsedGB';e={[math]::Round($_.Used/1GB)}}, @{n='FreeGB';e={[math]::Round($_.Free/1GB)}} | Format-Table -Auto
}
Section "SERVICES and the Java/JBoss process (command line masked)" {
  Get-WmiObject Win32_Service | Where { $_.State -eq 'Running' -or $_.Name -match 'jboss|java|tomcat|lrs|sql|oracle|postgres' } | Sort Name | Select Name, State, StartMode, StartName, PathName | Format-Table -Auto -Wrap
  Get-WmiObject Win32_Process | Where { $_.Name -match 'java|jboss|sqlservr|oracle|postgres' } | % { "{0} pid={1}`r`n  {2}" -f $_.Name, $_.ProcessId, (Mask $_.CommandLine) }
}
Section "NETWORK: listening ports and established connections (who talks to the Registry)" {
  netstat -ano | Select-String 'LISTENING|ESTABLISHED|TIME_WAIT'
}
Section "INSTALLED SOFTWARE" {
  $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
  Get-ItemProperty $k | Where { $_.DisplayName } | Sort DisplayName | Select DisplayName, DisplayVersion, Publisher, InstallDate | Format-Table -Auto
}
Section "LRS APPLICATION: JBoss/Java homes, deployments, archives" {
  # Locate the application server from what is actually running (never a drive-wide crawl: the
  # Registry keeps scanned documents in folders that can hold millions of files).
  $hints = @()
  $hints += Get-WmiObject Win32_Process | Where { $_.Name -match 'java|jboss|tomcat|wrapper' } | % { $_.CommandLine; $_.ExecutablePath }
  $hints += Get-WmiObject Win32_Service | Where { $_.PathName -match '(?i)jboss|tomcat|wrapper|java' } | % { $_.PathName }
  $found = New-Object System.Collections.Generic.List[string]
  foreach ($h in ($hints | Where { $_ })) {
    foreach ($rx in '(?i)-Djboss\.home\.dir="?([A-Za-z]:\\[^"]+?)"?(?:\s|$)', '(?i)-Dcatalina\.(?:home|base)="?([A-Za-z]:\\[^"]+?)"?(?:\s|$)', '(?i)([A-Za-z]:\\[^";]*?)\\bin\\[^\\";]*\.(?:jar|exe|bat)', '(?i)([A-Za-z]:\\[^";]*?)\\(?:server|standalone|webapps)\\') {
      foreach ($m in [regex]::Matches($h, $rx)) { $found.Add($m.Groups[$m.Groups.Count - 1].Value.TrimEnd('\')) }
    }
  }
  if ($found.Count -eq 0) {
    # Fallback: application-server folder names only, top 3 levels, never "lrs*" data folders.
    Shallow $drives @('jboss*','wildfly*','*tomcat*') -Dirs | Select -First 10 | % { $found.Add($_.FullName) }
  }
  $script:homes = @($found | Where { Test-Path $_ } | Sort -Unique)
  "application server homes: " + ($script:homes -join '; ')
  Write-Host ("            homes: " + ($script:homes -join '; ')) -ForegroundColor DarkGray
  # Only the server's own deploy / config / log folders, capped.
  $sub = 'server\*\deploy','server\*\conf','server\*\log','standalone\deployments','standalone\configuration','standalone\log','conf','webapps','logs','bin'
  $script:files = @(foreach ($h in $script:homes) { foreach ($d in $sub) {
    Get-ChildItem (Join-Path $h $d) -Recurse -File -ErrorAction SilentlyContinue |
      Where { $_.FullName -notmatch '\\(work|tmp|data)\\' }
  } }) | Select -First 20000
  "files indexed under those folders: $(@($script:files).Count)"
  $script:arch = @($script:files | Where { $_.Extension -in '.ear','.war' -or $_.Name -like '*-ds.xml' })
  $script:arch | Select FullName, Length, LastWriteTime | Format-Table -Auto -Wrap
}
Section "LRS APPLICATION: folder map of the application server (folders 4 levels, file counts)" {
  foreach ($h in ($script:homes | Where { $_ -notmatch '(?i)\\jdk|\\jre|Metabase' })) {
    foreach ($lvl in '*', '*\*', '*\*\*', '*\*\*\*') {
      Get-ChildItem (Join-Path $h $lvl) -Directory -ErrorAction SilentlyContinue | Where { $_.FullName -notmatch '\\(work|tmp|data)\\' } | % {
        "{0}  ({1} files)" -f $_.FullName, @(Get-ChildItem $_.FullName -File -ErrorAction SilentlyContinue).Count
      }
    }
  }
  # Any JBoss deploy folder, wherever the layout puts it, joins the indexed files.
  $deploy = foreach ($h in $script:homes) { foreach ($lvl in '*\deploy', '*\*\deploy', '*\*\*\deploy', '*\deployments', '*\*\deployments', '*\webapps', '*\*\webapps') { Get-ChildItem (Join-Path $h $lvl) -Directory -ErrorAction SilentlyContinue } }
  $more = @(foreach ($d in $deploy) { Get-ChildItem $d.FullName -Recurse -File -ErrorAction SilentlyContinue | Where { $_.FullName -notmatch '\\(work|tmp|data)\\' } }) | Select -First 20000
  $script:files = @($script:files + $more | Sort FullName -Unique)
  $script:arch = @($script:files | Where { $_.Extension -in '.ear','.war' -or $_.Name -like '*-ds.xml' })
  "deploy folders: " + (($deploy | % FullName) -join '; ')
  "files indexed now: $(@($script:files).Count); archives/datasources:"
  $script:arch | Select FullName, Length, LastWriteTime | Format-Table -Auto -Wrap
  # Logs live next to deploy (server\<config>\log); add them too.
  $logs = foreach ($h in $script:homes) { foreach ($lvl in '*\log', '*\*\log', '*\logs', '*\*\logs') { Get-ChildItem (Join-Path $h $lvl) -File -ErrorAction SilentlyContinue } }
  $script:files = @($script:files + @($logs) | Sort FullName -Unique)
}
Section "AUMENTUM REGISTRY desktop/server apps incl. CASHIER (payments): folders and config, masked" {
  $roots = foreach ($d in $drives) { foreach ($p in 'Program Files (x86)\Aumentum Registry', 'Program Files\Aumentum Registry', 'Program Files (x86)\Aumentum', 'Program Files\Aumentum', 'Program Files (x86)\Thomson Reuters', 'Program Files (x86)\Manatron') { $x = Join-Path $d $p; if (Test-Path $x) { $x } } }
  "Aumentum Registry folders: " + ($roots -join '; ')
  foreach ($r in $roots) {
    foreach ($lvl in '*', '*\*', '*\*\*') { Get-ChildItem (Join-Path $r $lvl) -Directory -ErrorAction SilentlyContinue | % { $_.FullName } }
    foreach ($lvl in '', '*\', '*\*\', '*\*\*\') {
      Get-ChildItem (Join-Path $r ($lvl + '*')) -File -Include *.config,*.xml,*.ini,*.properties,*.json -ErrorAction SilentlyContinue |
        Where { $_.Length -lt 300KB -and $_.Name -notmatch '(?i)\.resources\.|xmlserializers|^log4net' } | Select -First 80 | % {
          "----- $($_.FullName) ($($_.Length) bytes, $($_.LastWriteTime)) -----"; Mask ([IO.File]::ReadAllText($_.FullName))
        }
    }
    "### executables and versions"
    foreach ($lvl in '', '*\', '*\*\') { Get-ChildItem (Join-Path $r ($lvl + '*')) -File -Include *.exe,*.dll -ErrorAction SilentlyContinue | Select -First 300 | % { "{0}  {1}  {2}" -f $_.FullName.Substring($r.Length), $_.VersionInfo.FileVersion, $_.VersionInfo.ProductName } }
    "### recent logs (last 60 lines of the 5 newest, masked)"
    @(foreach ($lvl in '', '*\', '*\*\', '*\*\*\') { Get-ChildItem (Join-Path $r ($lvl + '*')) -File -Include *.log,*.txt -ErrorAction SilentlyContinue }) | Sort LastWriteTime -Descending | Select -First 5 | % {
      "----- $($_.FullName) ($($_.LastWriteTime)) -----"; Mask ((Get-Content $_.FullName -Tail 60) -join "`r`n")
    }
  }
}
Section "LRS APPLICATION: configuration files (datasources, web.xml, wsdd, properties) masked" {
  $script:files | Where { $_.Name -match '(-ds\.xml|^server\.xml|^web\.xml|^jboss-web\.xml|^server-config\.wsdd|\.properties|^login-config\.xml|^standalone.*\.xml)$' -and $_.Length -lt 400KB -and $_.FullName -notmatch '\\(docs|examples)\\' } |
    Select -First 150 | % { "----- $($_.FullName) ($($_.Length) bytes, $($_.LastWriteTime)) -----"; Mask ([IO.File]::ReadAllText($_.FullName)) }
  # Packed archives: read matching text entries in memory (nothing is extracted to disk).
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  foreach ($a in ($script:arch | Where { $_.Extension -in '.ear','.war' })) {
    $z = [IO.Compression.ZipFile]::OpenRead($a.FullName)
    try {
      foreach ($e in ($z.Entries | Where { $_.FullName -match '(web\.xml|server-config\.wsdd|\.properties|-ds\.xml|persistence\.xml|application\.xml)$' -and $_.Length -lt 400KB })) {
        "----- $($a.Name)!$($e.FullName) ($($e.Length) bytes) -----"
        $sr = New-Object IO.StreamReader($e.Open()); Mask $sr.ReadToEnd(); $sr.Close()
      }
      "### $($a.Name): $(@($z.Entries).Count) entries; jars: " + (($z.Entries | Where { $_.Name -match '\.jar$' } | % Name) -join ', ')
    } finally { $z.Dispose() }
  }
}
Section "SOAP SERVICES: Axis service list and WSDL (saved to lrs-capture\wsdl)" {
  $names = New-Object System.Collections.Generic.HashSet[string]
  foreach ($f in ($script:files | Where { $_.Name -eq 'server-config.wsdd' })) { ([regex]'<service\s+name="([^"]+)"').Matches([IO.File]::ReadAllText($f.FullName)) | % { [void]$names.Add($_.Groups[1].Value) } }
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  foreach ($a in ($script:arch | Where { $_.Extension -in '.ear','.war' })) {
    $z = [IO.Compression.ZipFile]::OpenRead($a.FullName)
    try { foreach ($e in ($z.Entries | Where { $_.FullName -match 'server-config\.wsdd$' })) { $sr = New-Object IO.StreamReader($e.Open()); ([regex]'<service\s+name="([^"]+)"').Matches($sr.ReadToEnd()) | % { [void]$names.Add($_.Groups[1].Value) }; $sr.Close() } } finally { $z.Dispose() }
  }
  "services found: " + ($names -join ', ')
  $wc = New-Object Net.WebClient
  foreach ($s in $names) {
    if ($s -match '^(AdminService|Version)$') { continue }
    $u = $LrsUrl.TrimEnd('/') + '/' + $s + '?wsdl'
    try { $x = $wc.DownloadString($u); [IO.File]::WriteAllText((Join-Path $cap "wsdl\$s.wsdl"), $x); "saved $s.wsdl ($($x.Length) chars) from $u" }
    catch { "could not fetch $u : $($_.Exception.Message)" }
  }
}
Section "JBOSS LOGS (last 300 lines of server.log / boot.log, masked)" {
  $script:files | Where { $_.Name -in 'server.log','boot.log','catalina.out' } | Sort LastWriteTime -Descending | Select -First 4 | % {
    "----- $($_.FullName) ($($_.LastWriteTime)) -----"; Mask ((Get-Content $_.FullName -Tail 300) -join "`r`n")
  }
}

# ---- Database: SQL Server through sqlcmd (Windows auth). Other engines: the datasource section
#      above names the engine and database; the report says so and the DB sections are skipped.
$sqlcmdPath = (Get-Command sqlcmd.exe -ErrorAction SilentlyContinue | Select -First 1).Path
if (-not $sqlcmdPath) { $sqlcmdPath = Get-ChildItem 'C:\Program Files\Microsoft SQL Server\*\Tools\Binn\SQLCMD.EXE','C:\Program Files (x86)\Microsoft SQL Server\*\Tools\Binn\SQLCMD.EXE' | Select -First 1 -Expand FullName }
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where { $_.Name -notmatch 'WID|FDLauncher|OLAP' -and $_.Status -eq 'Running' } | Select -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
function Sql([string]$db, [string]$q, [int]$width = -1) {
  # Read-only query through .NET SqlClient with the current Windows sign-in (works the same in a
  # local or a remote-management session; errors are printed instead of silently lost).
  # Output: a header line, then one line per row, '|'-separated. $width is accepted and ignored.
  try {
    # SQL login when one is given (the Registry's SQL Server does not accept the Windows account),
    # otherwise the current Windows sign-in. ApplicationIntent=ReadOnly documents the intent.
    $auth = if ($SqlUser) { "User ID=$SqlUser;Password=$SqlPassword" } else { 'Integrated Security=SSPI' }
    $cn = New-Object System.Data.SqlClient.SqlConnection ("Server=$SqlInstance;Database=$db;$auth;Application Name=readonly-inventory;ApplicationIntent=ReadOnly;Connect Timeout=15")
    $cn.Open()
    try {
      $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 600
      $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $q
      $r = $cmd.ExecuteReader()
      do {
        if ($r.FieldCount -gt 0) {
          (@(for ($i = 0; $i -lt $r.FieldCount; $i++) { $r.GetName($i) }) -join '|')
          while ($r.Read()) {
            (@(for ($i = 0; $i -lt $r.FieldCount; $i++) {
              if ($r.IsDBNull($i)) { 'NULL' } else { try { ([string]$r.GetValue($i)) -replace "[\r\n]+", ' ' } catch { '<unreadable>' } }
            }) -join '|')
          }
          ''
        }
      } while ($r.NextResult())
      $r.Close()
    } finally { $cn.Close() }
  } catch { "SQL ERROR ($db): $($_.Exception.Message)" }
}
$script:dbs = @()
Section "DATABASE: instance and databases" {
  "SQL instance: $SqlInstance   login: $(if ($SqlUser) { $SqlUser + ' (SQL login)' } else { 'Windows sign-in' })"
  Sql 'master' "SELECT @@SERVERNAME AS server, SERVERPROPERTY('ProductVersion') AS ver, SERVERPROPERTY('ProductLevel') AS lvl, SERVERPROPERTY('Edition') AS edition;
  SELECT d.name, d.state_desc, d.recovery_model_desc, d.create_date, CAST(SUM(f.size)*8/1024 AS int) AS size_mb FROM sys.databases d JOIN sys.master_files f ON f.database_id=d.database_id GROUP BY d.name,d.state_desc,d.recovery_model_desc,d.create_date ORDER BY d.name;
  SELECT database_name, type, MAX(backup_finish_date) AS last_backup FROM msdb.dbo.backupset GROUP BY database_name, type ORDER BY database_name;"
  $script:dbs = @(Sql 'master' "SELECT name FROM sys.databases WHERE database_id > 4 AND state_desc='ONLINE' ORDER BY name;" | Where { $_ -match '^\w+$' -and $_ -ne 'name' })
  "user databases: " + ($script:dbs -join ', ')
}
Section "DATABASE: per database - tables, row counts, columns, keys, code/settings tables, link columns" {
  foreach ($db in $script:dbs) {
    "################ DATABASE $db ################"
    "### tables and row counts"
    Sql $db "SELECT s.name+'.'+t.name AS tbl, SUM(p.rows) AS rows FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name ORDER BY s.name,t.name;"
    "### views, procedures, functions, triggers"
    Sql $db "SELECT s.name+'.'+o.name AS obj, o.type_desc FROM sys.objects o JOIN sys.schemas s ON s.schema_id=o.schema_id WHERE o.type IN ('V','P','FN','TF','IF','TR') ORDER BY o.type_desc, obj;"
    "### columns"
    Sql $db "SELECT TABLE_SCHEMA+'.'+TABLE_NAME AS tbl, COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH AS len, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION;"
    "### foreign keys"
    Sql $db "SELECT fk.name, OBJECT_SCHEMA_NAME(fk.parent_object_id)+'.'+OBJECT_NAME(fk.parent_object_id) AS child, OBJECT_SCHEMA_NAME(fk.referenced_object_id)+'.'+OBJECT_NAME(fk.referenced_object_id) AS parent FROM sys.foreign_keys fk ORDER BY child;"
    "### code / lookup tables (1-300 rows, TOP 100, masked)"
    $small = Sql $db "SELECT s.name+'.'+t.name FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) GROUP BY s.name,t.name HAVING SUM(p.rows) BETWEEN 1 AND 300 ORDER BY 1;"
    foreach ($t in ($small | Where { $_ -match '^\w+\.\w+$' } | Select -First 150)) { "## $t"; Mask ((Sql $db "SELECT TOP 100 * FROM $t;") -join "`r`n") }
    "### settings / configuration / integration tables (any size, TOP 500, masked)"
    $cfg = Sql $db "SELECT s.name+'.'+t.name FROM sys.tables t JOIN sys.schemas s ON s.schema_id=t.schema_id WHERE t.name LIKE '%setting%' OR t.name LIKE '%config%' OR t.name LIKE '%param%' OR t.name LIKE '%integrat%' OR t.name LIKE '%property_type%' OR t.name LIKE '%category%' OR t.name LIKE '%dictionary%' ORDER BY 1;"
    foreach ($t in ($cfg | Where { $_ -match '^\w+\.\w+$' })) { "## $t"; Mask ((Sql $db "SELECT TOP 500 * FROM $t;") -join "`r`n") }
    "### where the Cadastre integration keys are stored"
    $keys = 'MCIntegrationCategory','MCParcelFabricConfig','LinkageFields','LRSIDAttributeName','LinkStatusAttributeName','UniquenessValues','SearchValues','UpdatePropertyValues','BlockAttributeName','LotAttributeName','AreaUnitMatchValues','DeedPlanQueriesSetting','Resolution'
    $txtCols = Sql $db "SELECT c.TABLE_SCHEMA+'.'+c.TABLE_NAME+'|'+c.COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS c JOIN sys.tables t ON t.name=c.TABLE_NAME JOIN sys.partitions p ON p.object_id=t.object_id AND p.index_id IN (0,1) WHERE c.DATA_TYPE IN ('varchar','nvarchar','char','nchar') AND (c.CHARACTER_MAXIMUM_LENGTH BETWEEN 10 AND 400) GROUP BY c.TABLE_SCHEMA, c.TABLE_NAME, c.COLUMN_NAME HAVING SUM(p.rows) BETWEEN 1 AND 20000;"
    $inList = ($keys | % { "'$_'" }) -join ','
    foreach ($tc in ($txtCols | Where { $_ -match '^\w+\.\w+\|\w+$' })) {
      $t, $c = $tc.Split('|')
      $hit = Sql $db "SELECT TOP 1 1 FROM $t WHERE [$c] IN ($inList);"
      if ($hit -match '^\s*1\s*$') { "## keys found in $t.$c"; Mask ((Sql $db "SELECT * FROM $t;") -join "`r`n") }
    }
    "### link columns (UPIN / PIN / transaction / status / payment): counts and ranges only"
    $lc = Sql $db "SELECT TABLE_SCHEMA+'.'+TABLE_NAME+'|'+COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE COLUMN_NAME LIKE '%upin%' OR COLUMN_NAME LIKE '%pin' OR COLUMN_NAME LIKE '%pin_%' OR COLUMN_NAME LIKE '%trans%' OR COLUMN_NAME LIKE '%status%' OR COLUMN_NAME LIKE '%payment%' OR COLUMN_NAME LIKE '%receipt%' OR COLUMN_NAME LIKE '%fee%' OR COLUMN_NAME LIKE '%parcel%' ORDER BY 1;"
    foreach ($x in ($lc | Where { $_ -match '^\w+\.\w+\|\w+$' })) {
      $t, $c = $x.Split('|'); "## $t.$c"
      Sql $db "SELECT COUNT(*) AS rows, COUNT(DISTINCT [$c]) AS distinct_values, MIN([$c]) AS min_v, MAX([$c]) AS max_v FROM $t;"
      if ($c -match '(?i)status|type') { Sql $db "SELECT TOP 30 [$c] AS value, COUNT(*) AS n FROM $t GROUP BY [$c] ORDER BY n DESC;" }
    }
    "### activity by month (tables with a date column, last 24 months)"
    $dc = Sql $db "SELECT TOP 40 TABLE_SCHEMA+'.'+TABLE_NAME+'|'+COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE DATA_TYPE IN ('datetime','datetime2','date') AND (COLUMN_NAME LIKE '%creat%' OR COLUMN_NAME LIKE '%regist%' OR COLUMN_NAME LIKE '%complet%' OR COLUMN_NAME LIKE '%pay%') ORDER BY TABLE_NAME;"
    foreach ($x in ($dc | Where { $_ -match '^\w+\.\w+\|\w+$' })) {
      $t, $c = $x.Split('|'); "## $t.$c"
      Sql $db "SELECT CONVERT(char(7),[$c],120) AS ym, COUNT(*) AS n FROM $t WHERE [$c] >= DATEADD(month,-24,GETDATE()) GROUP BY CONVERT(char(7),[$c],120) ORDER BY 1;"
    }
  }
}
Section "END" { "Finished $(Get-Date). WSDL copies are in $cap\wsdl" }
Write-Host "`nDone. Report: $out`nWSDL:    $cap\wsdl" -ForegroundColor Green
