<#
  READ-ONLY export of Aumentum's application database (MCServer) on PRODUCTIONMC: the workflow
  engine (job types, tasks, task graph, steps, permissions), every job instance and task instance
  with its Registry transaction link, users/groups, the UPIN register and allocator, lineage,
  locks, the event log, dictionaries and integration query definitions. Phase 2 (workflow parity)
  and Phase 1 (complete UPIN reservation) are built from it.

  Every table is exported (Microsoft Sync bookkeeping tables sync_* excluded), one file each:
  <OutDir>\aumentum-export\mcserver\<schema>.<table>.psv, '|'-separated with a header line;
  empty field = NULL; newlines and '|' inside values are flattened to spaces.
  Never exported: password columns (password, pswd*) and binary values (only their byte length,
  as <column>__bytes). Only SELECT statements, READ UNCOMMITTED. PowerShell 4 compatible.
#>
param([string]$OutDir = (Join-Path $env:USERPROFILE 'Desktop'), [string]$SqlInstance = '', [string]$Database = 'MCServer',
      [string]$SqlUser = '', [string]$SqlPassword = '')
$ErrorActionPreference = 'Stop'
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where-Object { $_.Name -notmatch 'WID|SQLEXPRESS|FDLauncher|OLAP' -and $_.Status -eq 'Running' } | Select-Object -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
$dir = Join-Path (Join-Path $OutDir 'aumentum-export') 'mcserver'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$inv = [Globalization.CultureInfo]::InvariantCulture
$utf8 = New-Object Text.UTF8Encoding($false)
$auth = if ($SqlUser) { "User ID=$SqlUser;Password=$SqlPassword;TrustServerCertificate=True;" } else { 'Integrated Security=SSPI;' }
$connStr = "Server=$SqlInstance;Database=$Database;$($auth)Application Name=readonly-export;ApplicationIntent=ReadOnly;Connect Timeout=15"

function Clean($v) {
  if ($v -is [DBNull] -or $null -eq $v) { return '' }
  if ($v -is [double] -or $v -is [single]) { return $v.ToString('R', $inv) }
  if ($v -is [decimal]) { return $v.ToString($inv) }
  if ($v -is [datetime]) { return $v.ToString('yyyy-MM-ddTHH:mm:ss.fff', $inv) }
  if ($v -is [bool]) { if ($v) { return '1' } else { return '0' } }
  return ([string]$v) -replace '[\r\n|]+', ' '
}
function Query([string]$sql) {
  $cn = New-Object System.Data.SqlClient.SqlConnection $connStr; $cn.Open()
  try {
    $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 3600
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $sql
    $t = New-Object Data.DataTable; $t.Load($cmd.ExecuteReader())
    return ,$t   # the comma keeps PowerShell from unrolling the table into rows
  } finally { $cn.Close() }
}
function Export([string]$sql, [string]$file) {
  $cn = New-Object System.Data.SqlClient.SqlConnection $connStr; $cn.Open()
  $w = New-Object IO.StreamWriter((Join-Path $dir $file), $false, $utf8)
  $n = 0
  try {
    $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 3600
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $sql
    $r = $cmd.ExecuteReader()
    $w.WriteLine((@(for ($i = 0; $i -lt $r.FieldCount; $i++) { $r.GetName($i) }) -join '|'))
    while ($r.Read()) {
      $w.WriteLine((@(for ($i = 0; $i -lt $r.FieldCount; $i++) { Clean $r.GetValue($i) }) -join '|')); $n++
    }
    $r.Close()
  } finally { $w.Close(); $cn.Close() }
  return $n
}

$t0 = Get-Date
Write-Host "Exporting $Database from $SqlInstance (read-only) to $dir" -ForegroundColor Yellow
$tables = Query @"
SELECT s.name AS sch, t.name AS tbl FROM sys.tables t JOIN sys.schemas s ON s.schema_id = t.schema_id
WHERE t.name NOT LIKE 'sync[_]%' AND t.is_ms_shipped = 0 ORDER BY s.name, t.name
"@
$cols = Query @"
SELECT TABLE_SCHEMA AS sch, TABLE_NAME AS tbl, COLUMN_NAME AS col, DATA_TYPE AS typ, ORDINAL_POSITION AS pos
FROM INFORMATION_SCHEMA.COLUMNS ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION
"@
$summary = New-Object System.Collections.Generic.List[string]
$summary.Add("MCServer export of $env:COMPUTERNAME ($SqlInstance) at $(Get-Date -Format s)")
$i = 0
foreach ($t in $tables.Rows) {
  $i++
  $sel = foreach ($c in ($cols.Rows | Where-Object { $_.sch -eq $t.sch -and $_.tbl -eq $t.tbl } | Sort-Object pos)) {
    $q = '[' + $c.col + ']'
    if ($c.col -match '(?i)^(password|pswd)') { continue }                                  # never exported
    elseif ($c.typ -in 'binary', 'varbinary', 'image', 'timestamp', 'rowversion') { "DATALENGTH($q) AS [$($c.col)__bytes]" }
    elseif ($c.typ -in 'xml', 'text', 'ntext', 'sql_variant', 'hierarchyid') { "CAST($q AS nvarchar(max)) AS $q" }
    elseif ($c.typ -in 'geometry', 'geography') { "$q.STAsText() AS $q" }
    else { $q }
  }
  if (-not $sel) { continue }
  $name = "$($t.sch).$($t.tbl)"
  try {
    $n = Export ("SELECT " + ($sel -join ', ') + " FROM [$($t.sch)].[$($t.tbl)]") "$name.psv"
    Write-Host ("[{0,3}/{1}] {2}: {3:N0} rows" -f $i, $tables.Rows.Count, $name, $n) -ForegroundColor DarkGray
    $summary.Add("$name|$n")
  } catch {
    Write-Host ("[{0,3}/{1}] {2}: ERROR {3}" -f $i, $tables.Rows.Count, $name, $_.Exception.Message) -ForegroundColor Red
    $summary.Add("$name|ERROR $($_.Exception.Message)")
  }
}
$summary.Add("took $([int]((Get-Date) - $t0).TotalSeconds)s")
[IO.File]::WriteAllLines((Join-Path $dir '_summary.txt'), [string[]]$summary, $utf8)
Write-Host "`nDone. $($tables.Rows.Count) tables exported to $dir" -ForegroundColor Green
