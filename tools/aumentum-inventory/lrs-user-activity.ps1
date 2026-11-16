<#
  REGISTRY USER ACTIVITY - READ-ONLY. Run inside the Remote Desktop session on REGISTRY (10.10.10.5)
  via RUN-ME-USER-ACTIVITY.cmd. SELECT statements only (READ UNCOMMITTED); nothing is written.
  Lists what one Registry user worked on in a date range: workflow steps (lr_process_history),
  the transactions they touched, and their jBPM task instances.
  The database login is read from JBoss's own C:\LRSAS\server\default\deploy\ils-ds.xml and never printed.
  Writes <OutDir>\lrs-capture\user-activity-<user>-<from>-<to>.txt. PowerShell 3+.
#>
param([string]$User = 'psatdom', [datetime]$From = '2026-11-10', [datetime]$To = '2026-11-13',
      [string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path),
      [string]$DsFile = 'C:\LRSAS\server\default\deploy\ils-ds.xml')
$ErrorActionPreference = 'Stop'
if ($User -notmatch '^[A-Za-z0-9._@-]+$') { throw "Bad user name: $User" }
$dir = Join-Path $OutDir 'lrs-capture'; New-Item -ItemType Directory -Path $dir -Force | Out-Null
$out = Join-Path $dir ("user-activity-{0}-{1:yyyy-MM-dd}-{2:yyyy-MM-dd}.txt" -f $User, $From, $To)
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
"Registry activity of '$User' from $($From.ToString('yyyy-MM-dd')) to $($To.ToString('yyyy-MM-dd')) (exclusive), run $(Get-Date) on $env:COMPUTERNAME" | Out-File $out -Encoding UTF8

# Database login from the JBoss datasource (jdbc:jtds:sqlserver://HOST:PORT/DB)
[xml]$ds = Get-Content $DsFile
$d = $ds.datasources.'local-tx-datasource' | Select-Object -First 1
if ($d.'connection-url' -notmatch 'sqlserver://([^:/;]+)(?::(\d+))?/([^;]+)') { throw "Cannot read connection-url from $DsFile" }
$server = $Matches[1]; if ($Matches[2]) { $server += ",$($Matches[2])" }; $db = $Matches[3]
$cs = New-Object System.Data.SqlClient.SqlConnectionStringBuilder
$cs['Data Source'] = $server; $cs['Initial Catalog'] = $db; $cs['User ID'] = $d.'user-name'; $cs['Password'] = $d.password
$cs['Application Name'] = 'lrs-user-activity (read-only)'; $cs['Connect Timeout'] = 15
$con = New-Object System.Data.SqlClient.SqlConnection $cs.ConnectionString
$con.Open()
Log "Connected to $db on $server as $($d.'user-name')"

function Q([string]$title, [string]$sql) {
  Log ''; Log "===== $title =====" 'Cyan'
  try {
    $cmd = $con.CreateCommand(); $cmd.CommandTimeout = 300
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; $sql"
    [void]$cmd.Parameters.AddWithValue('@who', "%$User%")
    [void]$cmd.Parameters.AddWithValue('@from', $From)
    [void]$cmd.Parameters.AddWithValue('@to', $To)
    $t = New-Object System.Data.DataTable
    (New-Object System.Data.SqlClient.SqlDataAdapter $cmd).Fill($t) | Out-Null
    Log ("{0} rows" -f $t.Rows.Count)
    if ($t.Rows.Count) {
      Log (($t.Columns | ForEach-Object { $_.ColumnName }) -join ' | ')
      foreach ($r in $t.Rows) {
        Log (($r.ItemArray | ForEach-Object { if ($_ -is [datetime]) { $_.ToString('yyyy-MM-dd HH:mm:ss') } else { "$_" -replace '\s+', ' ' } }) -join ' | ')
      }
    }
  } catch { Log "ERROR: $($_.Exception.Message)" 'Red' }
}

# Every text column of lr_process_history is searched for the user, so the actor column name need not be known.
$cols = @(); $c = $con.CreateCommand()
$c.CommandText = "SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND TABLE_NAME='lr_process_history' AND DATA_TYPE IN ('varchar','nvarchar','char','nchar')"
$rd = $c.ExecuteReader(); while ($rd.Read()) { $cols += $rd.GetString(0) }; $rd.Close()
$match = if ($cols.Count) { '(' + (($cols | ForEach-Object { "CONVERT(nvarchar(4000),[$_]) LIKE @who" }) -join ' OR ') + ')' } else { '1=0' }
$inRange = '(task_create >= @from AND task_create < @to OR task_complete >= @from AND task_complete < @to)'
$matchH = if ($cols.Count) { '(' + (($cols | ForEach-Object { "CONVERT(nvarchar(4000),h.[$_]) LIKE @who" }) -join ' OR ') + ')' } else { '1=0' }
$inRangeH = '(h.task_create >= @from AND h.task_create < @to OR h.task_complete >= @from AND h.task_complete < @to)'

Q '1. Columns of lr_process_history' "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND TABLE_NAME='lr_process_history' ORDER BY ORDINAL_POSITION"
Q "2. Transactions $User touched (one line each, with PL file number)" "SELECT t.target_property_ids AS pl_file, t.transaction_number, h.process_name, t.create_date AS lodged, COUNT(*) AS steps, MIN(h.task_create) AS first_step, MAX(ISNULL(h.task_complete, h.task_create)) AS last_step, MAX(h.task_transition) AS sent_to FROM LRSAdmin.lr_process_history h WITH (NOLOCK) LEFT JOIN LRSAdmin.lr_transaction t WITH (NOLOCK) ON t.id = h.transaction_id WHERE $inRangeH AND $matchH GROUP BY t.target_property_ids, t.transaction_number, h.process_name, t.create_date ORDER BY MAX(ISNULL(h.task_complete, h.task_create))"
Q "3. Every workflow step of $User" "SELECT * FROM LRSAdmin.lr_process_history WITH (NOLOCK) WHERE $inRange AND $match ORDER BY task_create, id"
Q "4. jBPM task instances assigned to $User" @"
SELECT t.ID_, t.NAME_, t.ACTORID_, t.CREATE_, t.START_, t.END_, t.ISCANCELLED_, t.PROCINST_, p.NAME_ AS process, pi.KEY_ AS process_key
FROM LRSAdmin.JBPM_TASKINSTANCE t WITH (NOLOCK)
LEFT JOIN LRSAdmin.JBPM_PROCESSINSTANCE pi WITH (NOLOCK) ON pi.ID_ = t.PROCINST_
LEFT JOIN LRSAdmin.JBPM_PROCESSDEFINITION p WITH (NOLOCK) ON p.ID_ = pi.PROCESSDEFINITION_
WHERE t.ACTORID_ LIKE @who AND (t.CREATE_ >= @from AND t.CREATE_ < @to OR t.START_ >= @from AND t.START_ < @to OR t.END_ >= @from AND t.END_ < @to)
ORDER BY t.CREATE_
"@
Q '5. Spellings of user names like this one in the workflow (in case the login differs)' "SELECT ACTORID_, COUNT(*) AS tasks, MAX(CREATE_) AS latest FROM LRSAdmin.JBPM_TASKINSTANCE WITH (NOLOCK) WHERE ACTORID_ LIKE @who OR ACTORID_ LIKE '%satdom%' OR ACTORID_ LIKE '%psat%' GROUP BY ACTORID_ ORDER BY tasks DESC"
Q "7. Documents last changed by $User in the range, with PL file number" @"
SELECT t.target_property_ids AS pl_file, t.transaction_number, d.id AS document_id, d.document_number, d.document_type, d.create_date, d.modified_at, d.modified_by
FROM LRSAdmin.lr_source_document d WITH (NOLOCK)
LEFT JOIN LRSAdmin.lr_transaction_document td WITH (NOLOCK) ON td.document_id = d.id
LEFT JOIN LRSAdmin.lr_transaction t WITH (NOLOCK) ON t.id = td.transaction_id
WHERE d.modified_by LIKE @who AND (d.modified_at >= @from AND d.modified_at < @to OR d.create_date >= @from AND d.create_date < @to)
ORDER BY ISNULL(d.modified_at, d.create_date)
"@
Q "8. Notes written by $User in the range, with PL file number" "SELECT t.target_property_ids AS pl_file, t.transaction_number, n.create_date, n.task_name, n.comments FROM LRSAdmin.lr_transaction_note n WITH (NOLOCK) LEFT JOIN LRSAdmin.lr_transaction t WITH (NOLOCK) ON t.id = n.transaction_id WHERE n.user_name LIKE @who AND n.create_date >= @from AND n.create_date < @to ORDER BY n.create_date"
Q "9. All PL numbers $User touched in the range (workflow, documents, notes combined)" @"
SELECT pl_file, transaction_number, MIN(seen) AS first_seen, MAX(seen) AS last_seen, MIN(via) AS via FROM (
  SELECT t.target_property_ids AS pl_file, t.transaction_number, ISNULL(h.task_complete, h.task_create) AS seen, 'workflow' AS via
  FROM LRSAdmin.lr_process_history h WITH (NOLOCK) JOIN LRSAdmin.lr_transaction t WITH (NOLOCK) ON t.id = h.transaction_id
  WHERE h.actor LIKE @who AND (h.task_create >= @from AND h.task_create < @to OR h.task_complete >= @from AND h.task_complete < @to)
  UNION ALL
  SELECT t.target_property_ids, t.transaction_number, ISNULL(d.modified_at, d.create_date), 'document'
  FROM LRSAdmin.lr_source_document d WITH (NOLOCK) JOIN LRSAdmin.lr_transaction_document td WITH (NOLOCK) ON td.document_id = d.id JOIN LRSAdmin.lr_transaction t WITH (NOLOCK) ON t.id = td.transaction_id
  WHERE d.modified_by LIKE @who AND (d.modified_at >= @from AND d.modified_at < @to OR d.create_date >= @from AND d.create_date < @to)
  UNION ALL
  SELECT t.target_property_ids, t.transaction_number, n.create_date, 'note'
  FROM LRSAdmin.lr_transaction_note n WITH (NOLOCK) JOIN LRSAdmin.lr_transaction t WITH (NOLOCK) ON t.id = n.transaction_id
  WHERE n.user_name LIKE @who AND n.create_date >= @from AND n.create_date < @to
) x GROUP BY pl_file, transaction_number ORDER BY MIN(seen)
"@

$con.Close()
Log ''; Log "Finished $(Get-Date). Report: $out" 'Green'
