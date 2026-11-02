<#
  READ-ONLY export of the Aumentum Registry (LRS43, schema LRSAdmin) for building the new Cadastre's
  Registry linkage (Phase 5) against a COPY, never the live Registry. Runs ON the Registry server
  (10.10.10.5); lrs-export-remote.ps1 starts it from PRODUCTIONMC over remote management.

  Output in <OutDir> (copied back by lrs-export-remote.ps1):
    schema\lrs-tables.sql     CREATE TABLE for every LRSAdmin table (columns, types, identity, PK).
                              The 755 GB are document content (alf_*), so this is the whole
                              structure without the bulk: the copy is built from it, empty.
    schema\lrs-indexes.sql    the other indexes and unique constraints
    schema\lrs-fks.sql        the foreign keys (applied only where the exported rows allow it)
    schema\lrs-objects.sql    views, functions and procedures, as defined (sys.sql_modules)
    data\<table>.psv.gz       the rows of the linkage tables, gzip, UTF-8. Field separator ~|~ and
                              row end ~#~ + newline, so values keep their own '|' and line breaks.
                              First row: column names. NULL = empty field. Dates
                              yyyy-MM-dd HH:mm:ss.fff, floats round-trip ('R').
    app\...                   the LRS application's model definitions, read from disk: Hibernate
                              mappings (*.hbm.xml, *.cfg.xml), XML schemas (*.xsd, incl.
                              lrs-service-types.xsd, which the WSDLs import), WSDL/WSDD, and the
                              LRS jars themselves (to read the HQL entity -> table mapping that
                              Aumentum's LRSIntegrationQueryDefinition queries rely on). Config
                              text is masked (passwords, tickets).
    lrs-export-summary.txt    row counts per table, for reconciliation

  What is NOT exported (structure only): document content and Alfresco (alf_*, avm_*), the
  Registry's own workflow engine (JBPM_*, JMS_*), process history, and PERSONAL DATA - parties,
  persons, individuals, corporations, applicants, contacts, transaction parties - and the free-text
  legal fields of transactions (inscriptions, comments). Payments are exported without bank or
  account details. The new Cadastre reads names live through the Registry's own queries, as
  Aumentum does, so it never needs a copy of them.

  Changes nothing: SELECT statements (READ UNCOMMITTED, never blocks the Registry), catalog views,
  file reads. Writes only to -OutDir. PowerShell 4 compatible.
#>
param([string]$OutDir = (Join-Path $env:TEMP ('lrs-export-' + (Get-Date -Format yyyyMMdd-HHmmss))),
      [string]$SqlInstance = '', [string]$SqlUser = '', [string]$SqlPassword = '', [string]$Database = 'LRS43',
      [string]$AppRoot = 'C:\LRSAS\server\default')
$ErrorActionPreference = 'Stop'
foreach ($d in 'schema', 'data', 'app') { New-Item -ItemType Directory -Path (Join-Path $OutDir $d) -Force | Out-Null }
$inv = [Globalization.CultureInfo]::InvariantCulture
$utf8 = New-Object Text.UTF8Encoding($false)
$FieldSep = '~|~'; $RowEnd = "~#~`n"
if (-not $SqlInstance) {
  $svc = Get-Service 'MSSQL*' | Where { $_.Name -notmatch 'WID|SQLEXPRESS|FDLauncher|OLAP|Launchpad' -and $_.Status -eq 'Running' } | Select -First 1
  $SqlInstance = if (-not $svc -or $svc.Name -eq 'MSSQLSERVER') { $env:COMPUTERNAME } else { "$env:COMPUTERNAME\" + $svc.Name.Split('$')[1] }
}
$log = New-Object Collections.Generic.List[string]
function Say([string]$m, [string]$c = 'Gray') { Write-Host $m -ForegroundColor $c; $log.Add($m) }
function Mask($s) {
  $s -replace '(?i)((password|pwd|passwd|secret|apikey|api_key|token|ticket)\s*[=:]\s*)[^;"''<\s&]+', '$1****' `
     -replace '(?i)(<(password|pwd|passwd|secret|ticket)[^>]*>)[^<]*', '$1****' `
     -replace '(?i)((password|pwd|passwd|secret|ticket)"\s*value\s*=\s*")[^"]*', '$1****' `
     -replace '(?i)(name="(password|pwd|passwd|secret)"[^>]*>)[^<]*', '$1****'
}
function Conn {
  $auth = if ($SqlUser) { "User ID=$SqlUser;Password=$SqlPassword" } else { 'Integrated Security=SSPI' }
  $cn = New-Object System.Data.SqlClient.SqlConnection ("Server=$SqlInstance;Database=$Database;$auth;Application Name=readonly-export;ApplicationIntent=ReadOnly;Connect Timeout=15")
  $cn.Open(); $cn
}
function Query([string]$sql) {
  $cn = Conn
  try {
    $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 3600
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $sql
    $t = New-Object Data.DataTable; $t.Load($cmd.ExecuteReader()); return ,$t
  } finally { $cn.Close() }
}
function Cell($r, [int]$i) {
  if ($r.IsDBNull($i)) { return '' }
  $v = $r.GetValue($i)
  if ($v -is [double] -or $v -is [single]) { return $v.ToString('R', $inv) }
  if ($v -is [decimal]) { return $v.ToString($inv) }
  if ($v -is [datetime]) { return $v.ToString('yyyy-MM-dd HH:mm:ss.fff', $inv) }
  if ($v -is [bool]) { return $(if ($v) { '1' } else { '0' }) }
  if ($v -is [byte[]]) { return [BitConverter]::ToString($v).Replace('-', '') }
  if ($v -is [guid]) { return $v.ToString() }
  return [string]$v
}
# Stream one SELECT into data\<name>.psv.gz; returns the row count.
function Export([string]$name, [string]$sql) {
  $path = Join-Path $OutDir "data\$name.psv.gz"
  $cn = Conn
  $file = [IO.File]::Create($path); $gz = New-Object IO.Compression.GZipStream($file, [IO.Compression.CompressionMode]::Compress)
  $w = New-Object IO.StreamWriter($gz, $utf8); $n = 0
  try {
    $cmd = $cn.CreateCommand(); $cmd.CommandTimeout = 3600
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; " + $sql
    $r = $cmd.ExecuteReader()
    $w.Write((@(for ($i = 0; $i -lt $r.FieldCount; $i++) { $r.GetName($i) }) -join $FieldSep) + $RowEnd)
    while ($r.Read()) {
      $w.Write((@(for ($i = 0; $i -lt $r.FieldCount; $i++) { Cell $r $i }) -join $FieldSep) + $RowEnd); $n++
      if ($n % 20000 -eq 0) { Write-Host ("            {0}: {1:N0} rows" -f $name, $n) -ForegroundColor DarkGray }
    }
    $r.Close()
  } finally { $w.Close(); $cn.Close() }
  return $n
}

$t0 = Get-Date
Say "Registry export from $SqlInstance / $Database (read-only) to $OutDir" 'Yellow'

# ---------------------------------------------------------------- 1. structure
Say "[1/4] structure of every LRSAdmin table ..." 'Cyan'
$typeExpr = @"
CASE WHEN ty.name IN ('nvarchar','nchar') THEN ty.name + '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')'
     WHEN ty.name IN ('varchar','char','varbinary','binary') THEN ty.name + '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')'
     WHEN ty.name IN ('decimal','numeric') THEN ty.name + '(' + CAST(c.precision AS varchar(5)) + ',' + CAST(c.scale AS varchar(5)) + ')'
     WHEN ty.name IN ('datetime2','time','datetimeoffset') THEN ty.name + '(' + CAST(c.scale AS varchar(5)) + ')'
     ELSE ty.name END
"@
$cols = Query @"
SELECT s.name AS sch, t.name AS tbl, c.column_id, c.name AS col, $typeExpr AS typ, c.is_nullable,
       ic.seed_value, ic.increment_value, cc.definition AS computed
FROM sys.tables t JOIN sys.schemas s ON s.schema_id = t.schema_id
JOIN sys.columns c ON c.object_id = t.object_id
JOIN sys.types ty ON ty.user_type_id = c.user_type_id
LEFT JOIN sys.identity_columns ic ON ic.object_id = c.object_id AND ic.column_id = c.column_id
LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id
WHERE t.is_ms_shipped = 0 ORDER BY s.name, t.name, c.column_id
"@
$keys = Query @"
SELECT s.name AS sch, t.name AS tbl, i.name AS idx, i.is_primary_key, i.is_unique, i.is_unique_constraint, i.type_desc,
       ic.key_ordinal, ic.is_descending_key, ic.is_included_column, c.name AS col
FROM sys.indexes i JOIN sys.tables t ON t.object_id = i.object_id JOIN sys.schemas s ON s.schema_id = t.schema_id
JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
WHERE t.is_ms_shipped = 0 AND i.type > 0 ORDER BY s.name, t.name, i.name, ic.is_included_column, ic.key_ordinal, ic.index_column_id
"@
$fks = Query @"
SELECT fk.name AS fk, s.name AS sch, t.name AS tbl, rs.name AS rsch, rt.name AS rtbl, fkc.constraint_column_id AS ord,
       c.name AS col, rc.name AS rcol, fk.delete_referential_action_desc AS on_delete
FROM sys.foreign_keys fk
JOIN sys.tables t ON t.object_id = fk.parent_object_id JOIN sys.schemas s ON s.schema_id = t.schema_id
JOIN sys.tables rt ON rt.object_id = fk.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id
JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
JOIN sys.columns c ON c.object_id = fkc.parent_object_id AND c.column_id = fkc.parent_column_id
JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
ORDER BY fk.name, fkc.constraint_column_id
"@
$mods = Query "SELECT s.name AS sch, o.name AS obj, o.type_desc, m.definition FROM sys.sql_modules m JOIN sys.objects o ON o.object_id = m.object_id JOIN sys.schemas s ON s.schema_id = o.schema_id WHERE o.is_ms_shipped = 0 ORDER BY o.type_desc, s.name, o.name"

$q = { param($n) '[' + $n.Replace(']', ']]') + ']' }
$ddl = New-Object Text.StringBuilder
[void]$ddl.AppendLine("-- LRS43 structure exported $(Get-Date -Format s) from $SqlInstance. Tables only; see lrs-indexes.sql and lrs-fks.sql.")
foreach ($sch in ($cols | Select -Expand sch -Unique)) { [void]$ddl.AppendLine("IF SCHEMA_ID('$sch') IS NULL EXEC('CREATE SCHEMA " + (& $q $sch) + "');") }
[void]$ddl.AppendLine('GO')
$tables = $cols | Group-Object { $_.sch + '.' + $_.tbl }
foreach ($g in $tables) {
  $first = $g.Group[0]; $full = (& $q $first.sch) + '.' + (& $q $first.tbl)
  $lines = foreach ($c in $g.Group) {
    if ($c.computed -isnot [DBNull]) { "  $(& $q $c.col) AS $($c.computed)" }
    else {
      $id = if ($c.seed_value -isnot [DBNull]) { " IDENTITY($($c.seed_value),$($c.increment_value))" } else { '' }
      "  $(& $q $c.col) $($c.typ)$id $(if ($c.is_nullable) { 'NULL' } else { 'NOT NULL' })"
    }
  }
  $pk = $keys | Where { $_.sch -eq $first.sch -and $_.tbl -eq $first.tbl -and $_.is_primary_key -and -not $_.is_included_column }
  if ($pk) {
    $pkName = ($pk | Select -First 1).idx
    $lines += "  CONSTRAINT $(& $q $pkName) PRIMARY KEY $((($pk | Select -First 1).type_desc -replace '_', ' ' -replace ' COLUMNSTORE', '')) (" + ((($pk | % { (& $q $_.col) + $(if ($_.is_descending_key) { ' DESC' } else { '' }) })) -join ', ') + ")"
  }
  [void]$ddl.AppendLine("CREATE TABLE $full (`r`n" + ($lines -join ",`r`n") + "`r`n);")
}
[IO.File]::WriteAllText((Join-Path $OutDir 'schema\lrs-tables.sql'), $ddl.ToString(), $utf8)

$ix = New-Object Text.StringBuilder
foreach ($g in ($keys | Where { -not $_.is_primary_key } | Group-Object { $_.sch + '.' + $_.tbl + '.' + $_.idx })) {
  $f = $g.Group[0]; $full = (& $q $f.sch) + '.' + (& $q $f.tbl)
  $k = ($g.Group | Where { -not $_.is_included_column } | % { (& $q $_.col) + $(if ($_.is_descending_key) { ' DESC' } else { '' }) }) -join ', '
  $inc = ($g.Group | Where { $_.is_included_column } | % { & $q $_.col }) -join ', '
  if ($f.is_unique_constraint) { [void]$ix.AppendLine("ALTER TABLE $full ADD CONSTRAINT $(& $q $f.idx) UNIQUE ($k);") }
  else { [void]$ix.AppendLine("CREATE $(if ($f.is_unique) { 'UNIQUE ' } else { '' })$($f.type_desc -replace '_', ' ') INDEX $(& $q $f.idx) ON $full ($k)$(if ($inc) { " INCLUDE ($inc)" } else { '' });") }
}
[IO.File]::WriteAllText((Join-Path $OutDir 'schema\lrs-indexes.sql'), $ix.ToString(), $utf8)

$fk = New-Object Text.StringBuilder
foreach ($g in ($fks | Group-Object fk)) {
  $f = $g.Group[0]
  $od = if ($f.on_delete -ne 'NO_ACTION') { ' ON DELETE ' + ($f.on_delete -replace '_', ' ') } else { '' }
  [void]$fk.AppendLine("ALTER TABLE $(& $q $f.sch).$(& $q $f.tbl) WITH NOCHECK ADD CONSTRAINT $(& $q $f.fk) FOREIGN KEY (" + (($g.Group | % { & $q $_.col }) -join ', ') + ") REFERENCES $(& $q $f.rsch).$(& $q $f.rtbl) (" + (($g.Group | % { & $q $_.rcol }) -join ', ') + ")$od;")
}
[IO.File]::WriteAllText((Join-Path $OutDir 'schema\lrs-fks.sql'), $fk.ToString(), $utf8)

$ob = New-Object Text.StringBuilder
foreach ($m in $mods) { [void]$ob.AppendLine("-- $($m.type_desc) $($m.sch).$($m.obj)"); [void]$ob.AppendLine([string]$m.definition); [void]$ob.AppendLine('GO') }
[IO.File]::WriteAllText((Join-Path $OutDir 'schema\lrs-objects.sql'), $ob.ToString(), $utf8)
Say ("            {0} tables, {1} indexes, {2} foreign keys, {3} views/functions/procedures" -f $tables.Count, ($keys | Select -Expand idx -Unique).Count, ($fks | Select -Expand fk -Unique).Count, $mods.Rows.Count)

# ---------------------------------------------------------------- 2. rows of the linkage tables
Say "[2/4] rows of the linkage tables ..." 'Cyan'
$S = 'LRSAdmin'
# Configuration and code tables: every column, every row.
$config = 'lr_dictionary', 'lr_setting', 'lr_simplecounter', 'lr_extension_link', 'lr_registry', 'lr_registry_office',
          'lr_business_calendar', 'lr_fee_structure', 'lr_formfield', 'lr_transaction_metadata', 'lr_rrr_metadata'
# Linkage data: every row, without free-text columns (notes and comments can hold names).
$data = 'lr_spatial_unit', 'lr_spatial_unit_ext', 'lr_parcel',                        # properties: UPIN, SuID, refnum
        'lr_launit', 'lr_launit_spatial_unit', 'lr_launit_lineage',                  # legal units and their history
        'lr_transaction', 'lr_transaction_launit', 'lr_transaction_ext',             # transactions <-> properties
        'lr_transaction_document',                                                   # document index (ids only)
        'lr_mc_job',                                                                 # Cadastre job <-> transaction
        'lr_batch', 'lr_tempcounter', 'lr_rejection', 'lr_rejection_reason',
        'lr_payment', 'lr_payment_transaction'                                       # paid or not, receipts, amounts
$freeText = 'comments', 'note', 'description', 'extended_comments', 'inscription', 'inscription2', 'special_condition',
            'recommendation_memorandum', 'inspection_officer', 'reg_initials', 'data_corrected_comments', 'decision',
            'attachments', 'registrar',
            # payments: no bank or account details
            'bank_type', 'bank_name', 'account_name', 'account_number', 'pm1', 'pm2', 'pmt1', 'pmt2'
function Exists([string]$t) { (Query "SELECT CASE WHEN OBJECT_ID('$S.$t') IS NULL THEN 0 ELSE 1 END AS x").Rows[0].x -eq 1 }
function ColsOf([string]$t, [string[]]$skip) {
  $r = Query "SELECT c.name FROM sys.columns c WHERE c.object_id = OBJECT_ID('$S.$t') ORDER BY c.column_id"
  (@($r | % { $_.name } | Where { $skip -notcontains $_ } | % { '[' + $_ + ']' })) -join ', '
}
$counts = [ordered]@{}
foreach ($t in @($config) + @($data)) {
  if (-not (Exists $t)) { Say "            $t : not in this database, skipped" 'DarkYellow'; continue }
  $skip = if ($config -contains $t) { @() } else { $freeText }
  $counts[$t] = Export $t "SELECT $(ColsOf $t $skip) FROM $S.$t"
  Say ("            {0,-28} {1,9:N0} rows" -f $t, $counts[$t])
}
# Addresses of properties only (lr_address also holds people's addresses).
$counts['lr_address'] = Export 'lr_address' "SELECT a.* FROM $S.lr_address a WHERE a.id IN (SELECT su.address FROM $S.lr_spatial_unit su WHERE su.address IS NOT NULL)"
Say ("            {0,-28} {1,9:N0} rows (property addresses only)" -f 'lr_address', $counts['lr_address'])

# ---------------------------------------------------------------- 3. application model definitions
Say "[3/4] LRS application model definitions (Hibernate mappings, schemas, jars) ..." 'Cyan'
$appOut = Join-Path $OutDir 'app'; $nApp = 0
$roots = @($AppRoot) + @("$AppRoot\deploy", "$AppRoot\tmp\deploy", "$AppRoot\conf") | Where { Test-Path $_ } | Select -Unique
$want = '\.hbm\.xml$|\.cfg\.xml$|\.xsd$|\.wsdl$|\.wsdd$|processdefinition\.xml$|faces-config\.xml$|web\.xml$|jboss-web\.xml$|-ds\.xml$|\.properties$'
$seen = @{}
foreach ($root in $roots) {
  foreach ($f in (Get-ChildItem $root -Recurse -File -ErrorAction SilentlyContinue)) {
    if ($seen.ContainsKey($f.FullName)) { continue }; $seen[$f.FullName] = 1
    $rel = $f.FullName.Substring(2).TrimStart('\')   # C:\LRSAS\... -> LRSAS\...
    if ($f.Name -match $want -and $f.Length -lt 20MB -and $f.FullName -notmatch '\\log\\') {
      $dest = Join-Path $appOut $rel; New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
      if ($f.Name -match '\.(properties|xml|wsdd)$' -and $f.Name -notmatch '\.(hbm|cfg)\.xml$|\.xsd$') { [IO.File]::WriteAllText($dest, (Mask ([IO.File]::ReadAllText($f.FullName))), $utf8) }
      else { Copy-Item $f.FullName $dest }
      $nApp++
    }
    elseif ($f.Extension -match '^\.(jar|war|ear)$' -and $f.Name -match '(?i)lrs|ils|landsystems|integration|model|domain|manatron|aumentum' -and $f.Length -lt 60MB) {
      $dest = Join-Path $appOut $rel; New-Item -ItemType Directory -Path (Split-Path $dest -Parent) -Force | Out-Null
      Copy-Item $f.FullName $dest; $nApp++
    }
  }
}
Say "            $nApp files"

# ---------------------------------------------------------------- 4. summary
Say "[4/4] summary ..." 'Cyan'
$sum = @("Registry export of $env:COMPUTERNAME ($SqlInstance / $Database) at $(Get-Date -Format s), took $([int]((Get-Date) - $t0).TotalSeconds)s",
         "format: field separator '$FieldSep', row end '~#~' + LF, first row = column names, empty = NULL", '',
         'table|rows_exported|rows_in_database')
foreach ($k in $counts.Keys) {
  $all = (Query "SELECT COUNT_BIG(*) AS n FROM $S.$k").Rows[0].n
  $sum += "$k|$($counts[$k])|$all"
}
$sum += ''; $sum += "transaction statuses (lr_transaction.transaction_status -> lr_dictionary):"
foreach ($r in (Query "SELECT t.transaction_status AS id, d.code, d.label, COUNT(*) AS n FROM $S.lr_transaction t LEFT JOIN $S.lr_dictionary d ON d.Id = t.transaction_status GROUP BY t.transaction_status, d.code, d.label ORDER BY 1")) { $sum += "  $($r.id)|$($r.code)|$($r.label)|$($r.n)" }
$sum += ''; $sum += "Cadastre jobs by status (lr_mc_job.job_status):"
foreach ($r in (Query "SELECT job_status, COUNT(*) AS n, MIN(job_started) AS first_started, MAX(job_started) AS last_started FROM $S.lr_mc_job GROUP BY job_status ORDER BY 1")) { $sum += "  $($r.job_status)|$($r.n)|$($r.first_started)|$($r.last_started)" }
$sum += ''; $sum += $log
[IO.File]::WriteAllLines((Join-Path $OutDir 'lrs-export-summary.txt'), [string[]]$sum, $utf8)
Say "Done in $([int]((Get-Date) - $t0).TotalSeconds)s. Output: $OutDir" 'Green'
