<#
  REGISTRY TRANSACTION STATUS - READ-ONLY. Run inside the Remote Desktop session on REGISTRY (10.10.10.5)
  via RUN-ME-TRANSACTION-STATUS.cmd. SELECT statements only (READ UNCOMMITTED); nothing is written.
  For a list of transaction ids it shows: the transaction (number, type, status, dates), its whole
  workflow history, where it sits now (open jBPM task and who holds it), its documents and notes.
  Used to decide how to bring files back after the index trouble of 12 Nov 2026.
  The database login is read from JBoss's own ils-ds.xml and never printed.
  Writes <OutDir>\lrs-capture\transaction-status-<time>.txt. PowerShell 3+.
#>
param([string]$Ids = '10000000071365,10000000118260,10000000143500,10000000143503',
      [string]$OutDir = (Split-Path -Parent $MyInvocation.MyCommand.Path),
      [string]$DsFile = 'C:\LRSAS\server\default\deploy\ils-ds.xml')
$ErrorActionPreference = 'Stop'
$idList = @($Ids -split '[,; ]+' | Where-Object { $_ })
foreach ($i in $idList) { if ($i -notmatch '^\d+$') { throw "Not a numeric transaction id: $i" } }
$in = $idList -join ','
$dir = Join-Path $OutDir 'lrs-capture'; New-Item -ItemType Directory -Path $dir -Force | Out-Null
$out = Join-Path $dir ("transaction-status-{0}.txt" -f (Get-Date -Format 'yyyyMMdd-HHmm'))
function Log([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $s | Out-File $out -Append -Encoding UTF8 }
"Registry transaction status for $in, run $(Get-Date) on $env:COMPUTERNAME" | Out-File $out -Encoding UTF8

[xml]$ds = Get-Content $DsFile
$d = $ds.datasources.'local-tx-datasource' | Select-Object -First 1
if ($d.'connection-url' -notmatch 'sqlserver://([^:/;]+)(?::(\d+))?/([^;]+)') { throw "Cannot read connection-url from $DsFile" }
$server = $Matches[1]; if ($Matches[2]) { $server += ",$($Matches[2])" }; $db = $Matches[3]
$cs = New-Object System.Data.SqlClient.SqlConnectionStringBuilder
$cs['Data Source'] = $server; $cs['Initial Catalog'] = $db; $cs['User ID'] = $d.'user-name'; $cs['Password'] = $d.password
$cs['Application Name'] = 'lrs-transaction-status (read-only)'; $cs['Connect Timeout'] = 15
$con = New-Object System.Data.SqlClient.SqlConnection $cs.ConnectionString
$con.Open()
Log "Connected to $db on $server as $($d.'user-name')"

function Q([string]$title, [string]$sql) {
  Log ''; Log "===== $title =====" 'Cyan'
  try {
    $cmd = $con.CreateCommand(); $cmd.CommandTimeout = 300
    $cmd.CommandText = "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; $sql"
    $t = New-Object System.Data.DataTable
    (New-Object System.Data.SqlClient.SqlDataAdapter $cmd).Fill($t) | Out-Null
    Log ("{0} rows" -f $t.Rows.Count)
    if ($t.Rows.Count) {
      Log (($t.Columns | ForEach-Object { $_.ColumnName }) -join ' | ')
      foreach ($r in $t.Rows) {
        Log (($r.ItemArray | ForEach-Object { if ($_ -is [datetime]) { $_.ToString('yyyy-MM-dd HH:mm:ss') } else { $v = "$_" -replace '\s+', ' '; if ($v.Length -gt 300) { $v.Substring(0, 300) + '...' } else { $v } } }) -join ' | ')
      }
    }
  } catch { Log "ERROR: $($_.Exception.Message)" 'Red' }
}
$procs = "SELECT DISTINCT process_id FROM LRSAdmin.lr_process_history WITH (NOLOCK) WHERE transaction_id IN ($in)"

Q '1. The transactions' @"
SELECT t.id, t.transaction_number, t.lodgment_number, t.transaction_type, m.code AS type_code, m.label AS type_label, t.transaction_status,
       t.application_date, t.create_date, t.registration_date, t.complete_date, t.is_inactive, t.volume, t.folio, t.blockplot, t.boxno,
       t.version, t.begin_lifespan_version, t.end_lifespan_version, t.comments
FROM LRSAdmin.lr_transaction t WITH (NOLOCK)
LEFT JOIN LRSAdmin.lr_transaction_metadata m WITH (NOLOCK) ON m.Id = t.transaction_type
WHERE t.id IN ($in) ORDER BY t.id, t.version
"@
Q '2. Whole workflow history of these transactions (who did what, in order)' "SELECT transaction_id, process_id, task_id, task_name, actor, task_create, task_complete, task_transition FROM LRSAdmin.lr_process_history WITH (NOLOCK) WHERE transaction_id IN ($in) ORDER BY transaction_id, task_create, id"
Q '3. Where each file sits now: OPEN workflow tasks (no end date) and who holds them' @"
SELECT t.PROCINST_, t.ID_ AS task_id, t.NAME_ AS task, t.ACTORID_ AS held_by, t.CREATE_, t.START_, t.ISSUSPENDED_, t.ISCANCELLED_, t.ISOPEN_
FROM LRSAdmin.JBPM_TASKINSTANCE t WITH (NOLOCK)
WHERE t.PROCINST_ IN ($procs) AND t.END_ IS NULL ORDER BY t.PROCINST_, t.CREATE_
"@
Q '3b. Pooled actors (group queues) of those open tasks' @"
SELECT t.PROCINST_, t.ID_ AS task_id, t.NAME_ AS task, pa.ACTORID_ AS pooled_actor
FROM LRSAdmin.JBPM_TASKINSTANCE t WITH (NOLOCK)
JOIN LRSAdmin.JBPM_TASKACTORPOOL tap WITH (NOLOCK) ON tap.TASKINSTANCE_ = t.ID_
JOIN LRSAdmin.JBPM_POOLEDACTOR pa WITH (NOLOCK) ON pa.ID_ = tap.POOLEDACTOR_
WHERE t.PROCINST_ IN ($procs) AND t.END_ IS NULL ORDER BY t.PROCINST_
"@
Q '4. The workflow (process) instances: running, ended or suspended?' @"
SELECT pi.ID_, pi.KEY_, p.NAME_ AS process, pi.START_, pi.END_, pi.ISSUSPENDED_, tk.NODE_ AS current_node_id, n.NAME_ AS current_node
FROM LRSAdmin.JBPM_PROCESSINSTANCE pi WITH (NOLOCK)
LEFT JOIN LRSAdmin.JBPM_PROCESSDEFINITION p WITH (NOLOCK) ON p.ID_ = pi.PROCESSDEFINITION_
LEFT JOIN LRSAdmin.JBPM_TOKEN tk WITH (NOLOCK) ON tk.ID_ = pi.ROOTTOKEN_
LEFT JOIN LRSAdmin.JBPM_NODE n WITH (NOLOCK) ON n.ID_ = tk.NODE_
WHERE pi.ID_ IN ($procs) ORDER BY pi.ID_
"@
Q '5. Documents linked to the transactions (lr_transaction_document)' "SELECT transaction_id, document_id, document_index FROM LRSAdmin.lr_transaction_document WITH (NOLOCK) WHERE transaction_id IN ($in) ORDER BY transaction_id, document_index"
Q '5b. Document tables and their columns (to read the document rows next)' "SELECT TABLE_NAME, COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND (TABLE_NAME LIKE 'lr_document%' OR TABLE_NAME LIKE 'lr_source%' OR TABLE_NAME LIKE '%scan%') ORDER BY TABLE_NAME, ORDINAL_POSITION"
Q '5c. The document rows themselves (if the table is lr_document)' "IF OBJECT_ID('LRSAdmin.lr_document') IS NOT NULL SELECT d.* FROM LRSAdmin.lr_document d WITH (NOLOCK) WHERE d.id IN (SELECT document_id FROM LRSAdmin.lr_transaction_document WITH (NOLOCK) WHERE transaction_id IN ($in))"
Q '6. Notes on the transactions' "SELECT transaction_id, create_date, user_name, task_name, note_type, comments FROM LRSAdmin.lr_transaction_note WITH (NOLOCK) WHERE transaction_id IN ($in) ORDER BY transaction_id, create_date"
Q '7. Parcels (land units) and parties linked' "SELECT 'launit' AS kind, transaction_id, launit_id AS linked_id FROM LRSAdmin.lr_transaction_launit WITH (NOLOCK) WHERE transaction_id IN ($in) UNION ALL SELECT 'party', transaction_id, party_id FROM LRSAdmin.lr_transaction_party WITH (NOLOCK) WHERE transaction_id IN ($in) ORDER BY 2, 1"

$con.Close()
Log ''; Log "Finished $(Get-Date). Report: $out" 'Green'
