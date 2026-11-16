<#
  REGISTRY HIDDEN-FILES CHECKLIST - READ-ONLY. Run on REGISTRY (10.10.10.5) via RUN-ME-HIDDEN-FILES.cmd.
  SELECT statements only (READ UNCOMMITTED); nothing is written. Safe while LRSAS is rebuilding its index.
  Lists every file whose current workflow task was created in the power-cut window (default 9-11 Nov 2026):
  these are the ones the Transaction Queue cannot find until the index rebuild finishes.
  Use it as the test list after the rebuild: search each PL number under Target Property Number.
  The database login is read from JBoss's own ils-ds.xml and never printed.
  Writes <OutDir>\lrs-capture\hidden-files-<time>.txt (and a .csv). PowerShell 3+.
#>
param([datetime]$From = '2026-11-09', [datetime]$To = '2026-11-12',
      [string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path),
      [string]$DsFile = 'C:\LRSAS\server\default\deploy\ils-ds.xml')
$ErrorActionPreference = 'Stop'
$dir = Join-Path $OutDir 'lrs-capture'; New-Item -ItemType Directory -Path $dir -Force | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmm'
$out = Join-Path $dir "hidden-files-$stamp.txt"; $csv = Join-Path $dir "hidden-files-$stamp.csv"
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
"Files whose open workflow task was created $($From.ToString('yyyy-MM-dd')) to $($To.ToString('yyyy-MM-dd')) (exclusive), run $(Get-Date) on $env:COMPUTERNAME" | Out-File $out -Encoding UTF8

[xml]$ds = Get-Content $DsFile
$d = $ds.datasources.'local-tx-datasource' | Select-Object -First 1
if ($d.'connection-url' -notmatch 'sqlserver://([^:/;]+)(?::(\d+))?/([^;]+)') { throw "Cannot read connection-url from $DsFile" }
$server = $Matches[1]; if ($Matches[2]) { $server += ",$($Matches[2])" }; $db = $Matches[3]
$cs = New-Object System.Data.SqlClient.SqlConnectionStringBuilder
$cs['Data Source'] = $server; $cs['Initial Catalog'] = $db; $cs['User ID'] = $d.'user-name'; $cs['Password'] = $d.password
$cs['Application Name'] = 'lrs-hidden-files (read-only)'; $cs['Connect Timeout'] = 15
$con = New-Object System.Data.SqlClient.SqlConnection $cs.ConnectionString
$con.Open()

$cmd = $con.CreateCommand(); $cmd.CommandTimeout = 300
$cmd.CommandText = @"
SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED;
SELECT tr.target_property_ids AS pl_file, tr.transaction_number, ti.NAME_ AS waiting_at, ti.CREATE_ AS task_created,
       tr.create_date AS lodged, prev.actor AS previous_step_by, ISNULL(ti.ACTORID_, '') AS assigned_to,
       (SELECT COUNT(*) FROM LRSAdmin.JBPM_TASKACTORPOOL tap WITH (NOLOCK) WHERE tap.TASKINSTANCE_ = ti.ID_) AS offered_to_users
FROM LRSAdmin.JBPM_TASKINSTANCE ti WITH (NOLOCK)
CROSS APPLY (SELECT TOP 1 h.transaction_id FROM LRSAdmin.lr_process_history h WITH (NOLOCK) WHERE h.process_id = ti.PROCINST_) x
JOIN LRSAdmin.lr_transaction tr WITH (NOLOCK) ON tr.id = x.transaction_id
OUTER APPLY (SELECT TOP 1 h.actor FROM LRSAdmin.lr_process_history h WITH (NOLOCK)
             WHERE h.process_id = ti.PROCINST_ AND h.task_complete IS NOT NULL ORDER BY h.task_complete DESC) prev
WHERE ti.END_ IS NULL AND ti.ISCANCELLED_ = 0 AND ti.CREATE_ >= @from AND ti.CREATE_ < @to
ORDER BY ti.NAME_, ti.CREATE_
"@
[void]$cmd.Parameters.AddWithValue('@from', $From); [void]$cmd.Parameters.AddWithValue('@to', $To)
$t = New-Object System.Data.DataTable
(New-Object System.Data.SqlClient.SqlDataAdapter $cmd).Fill($t) | Out-Null
$con.Close()

Log ("{0} files are waiting on a task created in that window" -f $t.Rows.Count) 'Cyan'
$t.Rows | Group-Object waiting_at | ForEach-Object { Log ("  {0,-28} {1}" -f $_.Name, $_.Count) }
Log ''
Log 'pl_file | transaction_number | waiting_at | task_created | lodged | previous_step_by | assigned_to | offered_to_users'
foreach ($r in $t.Rows) {
  Log (($r.ItemArray | ForEach-Object { if ($_ -is [datetime]) { $_.ToString('yyyy-MM-dd HH:mm') } else { "$_" } }) -join ' | ')
}
$t | Export-Csv $csv -NoTypeInformation -Encoding UTF8
Log ''; Log "Finished $(Get-Date). Report: $out  (spreadsheet: $csv)" 'Green'
