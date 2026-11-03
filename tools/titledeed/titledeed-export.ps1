<#
  Export the title deed plan workflow (job type "Deed Plan Creationv2") from a Cadastre server.

  Writes a folder  <OutDir>\titledeed-workflow-<server>-<yyyyMMdd-HHmm>\  with:
    workflow.txt          the workflow, readable: tasks in flow order, where each can go next, the
                          steps of each task with their tools, and who may work each task
    workflow.json         the same definition as JSON (job type, tasks, steps, flow, statuses)
    tasks.csv             one row per task: id, name, steps, permitted principals
    flow.csv              the task graph: from task -> to task
    steps.csv             one row per task step: task, step, tool commands
    jobs.csv              every job of the type: id, name, file no., Registry transaction, status,
                          current task, assignee, created / updated / ended
    task-history.csv      the task instances of the jobs (open jobs by default; -AllHistory: all)
    summary.txt           counts by status and by current task

  READ-ONLY: only GET requests to the Cadastre server. PowerShell 4 compatible.

  Usage:  titledeed-export.cmd
          titledeed-export.cmd http://10.10.10.7:8080
          powershell -File titledeed-export.ps1 -Server http://10.10.10.9:8080 -OutDir D:\exports -AllHistory
#>
param(
  [string]$Server = 'http://10.10.10.9:8080',
  [string]$OutDir = '.',
  [string]$JobType = 'Deed Plan Creationv2',   # name or id
  [switch]$AllHistory                           # task history of every job (one request per job: slow)
)
$ErrorActionPreference = 'Stop'
$Server = $Server.TrimEnd('/')

function Get-Api([string]$path) {
  try { return Invoke-RestMethod -Uri ($Server + $path) -Method Get -TimeoutSec 120 }
  catch { throw "GET $path failed: $($_.Exception.Message)" }
}
function AsList($v) { if ($v -eq $null) { return @() } return @($v) }
function Csv($rows, [string]$name) {
  $p = Join-Path $out $name
  if (@($rows).Count) { $rows | Export-Csv -Path $p -NoTypeInformation -Encoding UTF8 }
  else { Set-Content -Path $p -Value '' -Encoding UTF8 }
  Write-Host ("  {0,-18} {1,7} rows" -f $name, @($rows).Count)
}

# --- definition -----------------------------------------------------------------------------------
Write-Host "Reading the workflow from $Server ..." -ForegroundColor Cyan
$null = Get-Api '/health'
$defs = Get-Api '/api/v1/wf/definitions'
$jt = AsList $defs.job_types | Where-Object { "$($_.id)" -eq $JobType -or $_.name -eq $JobType } | Select-Object -First 1
if (-not $jt) { throw "job type '$JobType' not found; job types: $((AsList $defs.job_types | ForEach-Object { "$($_.id) $($_.name)" }) -join ', ')" }
$taskById = @{}; foreach ($t in (AsList $defs.tasks)) { $taskById[[int]$t.id] = $t }
$statusByCode = @{}; foreach ($s in (AsList $defs.statuses)) { $statusByCode[[int]$s.code] = $s.label }
$principals = @{}
try { foreach ($p in (AsList (Get-Api '/api/v1/wf/principals'))) { $principals[[int]$p.sid] = $(if ($p.name) { $p.name } else { $p.code }) } } catch {}
function TaskName($id) { if ($id -eq $null) { return '(end)' } $t = $taskById[[int]$id]; if ($t) { return $t.name } return "task $id" }
function Who($sids) { return ((AsList $sids | ForEach-Object { if ($principals.ContainsKey([int]$_)) { "$($principals[[int]$_]) ($_)" } else { "sid $_" } }) -join '; ') }

# Tasks of this job type in the workflow's own order (task_order), start task first, end task last.
$flow = AsList $jt.flow
$rank = @{}
foreach ($e in $flow) { $k = [int]$e.task_id; $o = [int]$e.task_order; if (-not $rank.ContainsKey($k) -or $o -lt $rank[$k]) { $rank[$k] = $o } }
foreach ($e in $flow) { if ($e.next_task_id -ne $null -and -not $rank.ContainsKey([int]$e.next_task_id)) { $rank[[int]$e.next_task_id] = 100000 } }
$rank[[int]$jt.start_task_id] = -1
$rank[[int]$jt.end_task_id] = 1000000
$order = New-Object System.Collections.ArrayList
foreach ($k in ($rank.Keys | Sort-Object { $rank[$_] }, { $_ })) { [void]$order.Add([int]$k) }

$stamp = Get-Date -Format 'yyyyMMdd-HHmm'
$hostTag = ($Server -replace '^https?://', '') -replace '[^A-Za-z0-9.-]', '_'
$out = Join-Path (Resolve-Path $OutDir) "titledeed-workflow-$hostTag-$stamp"
New-Item -ItemType Directory -Path $out -Force | Out-Null
Write-Host "Writing to $out" -ForegroundColor Cyan

# workflow.txt (readable)
$txt = New-Object System.Collections.ArrayList
[void]$txt.Add("Workflow: $($jt.name)  (job type $($jt.id), active=$($jt.active))")
[void]$txt.Add("Exported from $Server on $(Get-Date -Format s)")
[void]$txt.Add("Start task: $(TaskName $jt.start_task_id)    End task: $(TaskName $jt.end_task_id)")
[void]$txt.Add('')
$n = 0
foreach ($id in $order) {
  $n++; $t = $taskById[$id]
  $nexts = @($flow | Where-Object { [int]$_.task_id -eq $id -and $_.next_task_id -ne $null } | ForEach-Object { TaskName $_.next_task_id })
  $tag = @(); if ($id -eq [int]$jt.start_task_id) { $tag += 'START' }; if ($id -eq [int]$jt.end_task_id) { $tag += 'END' }
  [void]$txt.Add(("{0}. {1} (task {2}){3}" -f $n, (TaskName $id), $id, $(if ($tag.Count) { '  [' + ($tag -join ', ') + ']' } else { '' })))
  [void]$txt.Add(("   next: {0}" -f $(if ($nexts.Count) { $nexts -join '  |  ' } else { '(none: the job completes here)' })))
  if ($t) {
    [void]$txt.Add(("   may work it: {0}" -f $(if ($t.permitted_sids) { Who $t.permitted_sids } else { '(anyone)' })))
    foreach ($st in (AsList $t.steps)) {
      $named = @(AsList $st.commands | Where-Object { -not "$_".StartsWith('{') })
      $guids = @(AsList $st.commands | Where-Object { "$_".StartsWith('{') }).Count
      [void]$txt.Add(("   - step {0} {1}: {2}{3}" -f $st.id, $st.name, ($named -join ', '), $(if ($guids) { "$(if ($named.Count) { ' + ' })$guids ArcMap/Aumentum tool(s)" } else { '' })))
    }
  }
  [void]$txt.Add('')
}
Set-Content -Path (Join-Path $out 'workflow.txt') -Value $txt -Encoding UTF8
Write-Host ("  {0,-18} {1,7} tasks" -f 'workflow.txt', $order.Count)

$defOut = New-Object PSObject -Property @{
  job_type = $jt; statuses = $defs.statuses
  tasks = @($order | ForEach-Object { $taskById[$_] } | Where-Object { $_ })
  exported_from = $Server; exported_at = (Get-Date -Format s)
}
Set-Content -Path (Join-Path $out 'workflow.json') -Value ($defOut | ConvertTo-Json -Depth 8) -Encoding UTF8
Write-Host ("  {0,-18}" -f 'workflow.json')

Csv @($order | ForEach-Object { $t = $taskById[$_]; New-Object PSObject -Property ([ordered]@{
  task_id = $_; task = (TaskName $_); is_start = ($_ -eq [int]$jt.start_task_id); is_end = ($_ -eq [int]$jt.end_task_id)
  steps = $(if ($t) { (AsList $t.steps | ForEach-Object { $_.name }) -join '; ' } else { '' })
  permitted = $(if ($t) { Who $t.permitted_sids } else { '' }) }) }) 'tasks.csv'
Csv @($flow | ForEach-Object { New-Object PSObject -Property ([ordered]@{
  from_task_id = $_.task_id; from_task = (TaskName $_.task_id); to_task_id = $_.next_task_id; to_task = (TaskName $_.next_task_id); task_order = $_.task_order }) }) 'flow.csv'
Csv @(foreach ($id in $order) { $t = $taskById[$id]; if ($t) { foreach ($st in (AsList $t.steps)) { New-Object PSObject -Property ([ordered]@{
  task_id = $id; task = $t.name; step_id = $st.id; step = $st.name; commands = ((AsList $st.commands) -join ' ') }) } } }) 'steps.csv'

# --- jobs -----------------------------------------------------------------------------------------
Write-Host "Reading the jobs ..." -ForegroundColor Cyan
$jobs = New-Object System.Collections.ArrayList
$offset = 0; $total = $null
do {
  $page = Get-Api ("/api/v1/wf/jobs?open=false&job_type={0}&limit=1000&offset={1}" -f $jt.id, $offset)
  $total = [int]$page.total
  foreach ($j in (AsList $page.jobs)) { [void]$jobs.Add($j) }
  $offset += 1000
  Write-Host ("  {0:N0} / {1:N0}" -f $jobs.Count, $total) -ForegroundColor DarkGray
} while ($jobs.Count -lt $total -and @($page.jobs).Count -gt 0)

Csv @($jobs | ForEach-Object { New-Object PSObject -Property ([ordered]@{
  job_id = $_.id; name = $_.name; file_no = $(if ($_.name -match '^Job-(.+)$') { $Matches[1] } else { '' })
  transaction = $_.lrs_transaction_number; status = $_.status; status_label = $_.status_label; is_open = $_.is_open
  current_task = $_.current_task; assigned_to = $_.assigned_to; priority = $_.priority
  created = $_.created; started = $_.started; updated = $_.updated; ended = $_.ended; source = $_.source; comment = $_.comment }) }) 'jobs.csv'

# --- task history ---------------------------------------------------------------------------------
$histJobs = if ($AllHistory) { @($jobs) } else { @($jobs | Where-Object { $_.is_open }) }
Write-Host ("Reading the task history of {0:N0} {1}job(s) ..." -f $histJobs.Count, $(if ($AllHistory) { '' } else { 'open ' })) -ForegroundColor Cyan
$hist = New-Object System.Collections.ArrayList
$i = 0; $failed = 0
foreach ($j in $histJobs) {
  $i++
  try {
    $d = Get-Api "/api/v1/wf/jobs/$($j.id)"
    foreach ($h in (AsList $d.history)) { [void]$hist.Add((New-Object PSObject -Property ([ordered]@{
      job_id = $j.id; job = $j.name; transaction = $j.lrs_transaction_number; task_instance_id = $h.id; task_id = $h.task_id; task = $h.task
      assigned_to = $h.assigned_to; assigned = $h.assigned; started = $h.started; ended = $h.ended; comment = $h.comment }))) }
  } catch { $failed++ }
  if ($i % 200 -eq 0) { Write-Host ("  {0:N0} / {1:N0}" -f $i, $histJobs.Count) -ForegroundColor DarkGray }
}
Csv $hist 'task-history.csv'
if ($failed) { Write-Host "  $failed job(s) could not be read" -ForegroundColor Yellow }

# --- summary --------------------------------------------------------------------------------------
$sum = New-Object System.Collections.ArrayList
[void]$sum.Add("$($jt.name) on $Server, exported $(Get-Date -Format s)")
[void]$sum.Add(("jobs: {0:N0}   open: {1:N0}   with a Registry transaction: {2:N0}" -f $jobs.Count, @($jobs | Where-Object { $_.is_open }).Count, @($jobs | Where-Object { $_.lrs_transaction_number }).Count))
[void]$sum.Add('')
[void]$sum.Add('by status:')
foreach ($g in ($jobs | Group-Object status_label | Sort-Object Count -Descending)) { [void]$sum.Add(("  {0,-32} {1,7:N0}" -f $g.Name, $g.Count)) }
[void]$sum.Add('')
[void]$sum.Add('open jobs by current task:')
foreach ($g in ($jobs | Where-Object { $_.is_open } | Group-Object current_task | Sort-Object Count -Descending)) { [void]$sum.Add(("  {0,-32} {1,7:N0}" -f $g.Name, $g.Count)) }
[void]$sum.Add('')
[void]$sum.Add(("task history: {0:N0} task instance(s) of {1:N0} {2}job(s)" -f $hist.Count, $histJobs.Count, $(if ($AllHistory) { '' } else { 'open ' })))
Set-Content -Path (Join-Path $out 'summary.txt') -Value $sum -Encoding UTF8
Write-Host ''
$sum | ForEach-Object { Write-Host $_ }
Write-Host ''
Write-Host "Done: $out" -ForegroundColor Green
