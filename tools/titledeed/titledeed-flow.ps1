<#
  Title deed plan flow: where is a Registry file (PL... or PLS...) in the new Cadastre?

  Walks every stage of the Deed Plan Creationv2 flow for one file and prints what is done and the
  next step:
    1. Registry file        transaction, status, property (SuID), UPIN the Registry holds
    2. Job                  the Cadastre job of the transaction, its task and task history
    3. Digitising           job area, draft parcels, corners and their beacon numbers
    4. Commit               the parcel(s) posted for the file and their UPIN
    5. Registry link        the link made in the Registry (LRSID, LinkageStatus)
    6. Title deed plan      the site plan, and plans sent to the Registry
    7. Job completion       completed / approved / rejected by the Registry

  READ-ONLY: only GET requests to the Cadastre server. Nothing is created, committed or sent.
  PowerShell 4 compatible (Windows Server 2012 R2).

  Usage:  titledeed-flow.cmd PL67363
          titledeed-flow.cmd PL67363 http://10.10.10.9:8080
          powershell -File titledeed-flow.ps1 -File PLS000138850 -Server http://10.10.10.7:8080
#>
param(
  [Parameter(Mandatory = $true, Position = 0)][string]$File,
  [Parameter(Position = 1)][string]$Server = 'http://10.10.10.9:8080'
)
$ErrorActionPreference = 'Stop'
$Server = $Server.TrimEnd('/')
$File = ($File -replace '\s', '').ToUpper()

function Get-Api([string]$path) {
  try { return Invoke-RestMethod -Uri ($Server + $path) -Method Get -TimeoutSec 60 }
  catch {
    $msg = $_.Exception.Message
    try { $r = $_.Exception.Response; if ($r) { $sr = New-Object IO.StreamReader($r.GetResponseStream()); $body = $sr.ReadToEnd(); if ($body) { $msg = "$msg $body" } } } catch {}
    throw "GET $path failed: $msg"
  }
}
function Esc([string]$s) { return [Uri]::EscapeDataString($s) }
function Head([string]$t) { Write-Host ''; Write-Host $t -ForegroundColor Cyan }
function Ok([string]$t)   { Write-Host "  [done] $t" -ForegroundColor Green }
function Todo([string]$t) { Write-Host "  [todo] $t" -ForegroundColor Yellow }
function Warn([string]$t) { Write-Host "  [!]    $t" -ForegroundColor Red }
function Info([string]$t) { Write-Host "         $t" }
function Val($v) { if ($v -eq $null -or "$v" -eq '') { return '-' } return "$v" }
function AsList($v) { if ($v -eq $null) { return @() } return @($v) }

$flow = @{ next = $null }   # the first step still to do (a hashtable, so functions can set it in any scope)
function SetNext([string]$t) { if (-not $flow.next) { $flow.next = $t } }

Write-Host "Title deed plan flow for $File on $Server  ($(Get-Date -Format 'yyyy-MM-dd HH:mm'))" -ForegroundColor White

# --- 0. Server and Registry connection ------------------------------------------------------------
Head '0. Server'
try { $null = Get-Api '/health'; Ok 'Cadastre server answers' } catch { Warn "Cadastre server not reachable: $($_.Exception.Message)"; exit 1 }
$reg = Get-Api '/api/v1/registry/status'
if ($reg.enabled) {
  Ok ("Registry connected: {0}" -f (Val $reg.url))
  if ($reg.last) { Info ("last link check {0}  ok={1}" -f (Val $reg.last.at), (Val $reg.last.ok)) }
  if ($reg.write_enabled) { Info 'writing to the Registry: ON  (Send plan really attaches the plan to the transaction)' }
  else { Info 'writing to the Registry: OFF (Send plan is refused on this server)' }
} else { Warn 'Registry features are off on this server (CADASTRE_REGISTRY_URL not set): Find file cannot work'; exit 1 }

# --- 1. Registry file -----------------------------------------------------------------------------
Head '1. Registry file'
$found = AsList (Get-Api ('/api/v1/registry/find?q=' + (Esc $File)))
if ($found.Count -eq 0) { Warn "No Registry transaction or file $File (type PL + number, or the full PLS number)"; exit 1 }
foreach ($f in $found) {
  Ok ("transaction {0}  status {1} ({2})  file {3}  property {4}" -f $f.transaction_number, (Val $f.status), (Val $f.status_code), (Val $f.property_number), (Val $f.su_id))
  if ($f.upin) { Info "UPIN held by the Registry property: $($f.upin)" } else { Info 'UPIN held by the Registry property: none yet (set when the Registry admin links)' }
}
if ($found.Count -gt 1) { Info "$($found.Count) transactions match; the first one with a job (else the first) is followed below" }
$f = $found | Where-Object { $_.job_instance_id } | Select-Object -First 1
$jobId = $null
if ($f) { $jobId = $f.job_instance_id }
else {
  # Find file reports open jobs only: look for the file's latest job (closed ones too), by its
  # transactions and by the job name Job-<file no>.
  $fileKey = if ($found[0].property_number) { $found[0].property_number } else { $File }
  $terms = @($found | ForEach-Object { $_.transaction_number }) + @("Job-$fileKey")
  $hits = @()
  foreach ($t in $terms) { $hits += AsList ((Get-Api ('/api/v1/wf/jobs?open=false&limit=50&q=' + (Esc $t))).jobs) }
  $hits = @($hits | Where-Object { ($terms -contains $_.lrs_transaction_number) -or $_.name -eq "Job-$fileKey" } | Sort-Object id -Unique | Sort-Object id -Descending)
  if ($hits.Count) {
    $jobId = $hits[0].id
    $f = $found | Where-Object { $_.transaction_number -eq $hits[0].lrs_transaction_number } | Select-Object -First 1
    if ($hits.Count -gt 1) { Info ("{0} jobs for this file: {1}; the latest (#{2}) is followed below" -f $hits.Count, (($hits | ForEach-Object { "#$($_.id) $($_.status_label)" }) -join ', '), $jobId) }
  }
}
if (-not $f) { $f = $found[0] }
if ($f.status -ne 'Active') { Warn "the transaction is $($f.status): Aumentum refuses to link or send plans for Withdrawn, Postponed, Completed or Rejected transactions" }
$fileNo = if ($f.property_number) { $f.property_number } else { $File }

# --- 2. Job ---------------------------------------------------------------------------------------
Head '2. Job'
$job = $null; $ed = $null
# Parcels already posted for this file (by the property number on their attribute record).
$cands = AsList (Get-Api ('/api/v1/parcels/search?number=' + (Esc $fileNo)))
$posted = @($cands | Where-Object { $_.status -eq 'posted' })
if (-not $jobId) {
  Todo 'no Cadastre job for this file yet'
  if ($posted.Count) {
    Info ("the file already has a committed parcel ({0}): its job takes the existing-parcel path, Search Cadastre for Parcels > Generate Title Deed Plan" -f (($posted | ForEach-Object { $_.parcel_number }) -join ', '))
    SetNext "In the editor: Registry tab > Find file > $File > Start digitising, then Send To Task > Generate Title Deed Plan (the parcel exists: no plotting)"
  } else {
    SetNext "In the editor: Registry tab > Find file > $File > Start digitising (creates Job-$fileNo, Deed Plan Creationv2)"
  }
} else {
  $d = Get-Api "/api/v1/wf/jobs/$jobId"
  $job = $d.job
  Ok ("job #{0} {1}  ({2})  status {3}" -f $job.id, $job.name, $job.job_type, $job.status_label)
  Info ("current task: {0}" -f (Val $job.current_task))
  $nt = AsList $d.next_tasks | ForEach-Object { $_.name }
  if ($nt) { Info ("next task(s): {0}" -f ($nt -join ', ')) }
  Info 'task history:'
  foreach ($h in (AsList $d.history)) { Info ("  {0,-30} assigned {1}  ended {2}" -f $h.task, (Val $h.assigned), (Val $h.ended)) }
  if ($job.lrs_transaction_number -ne $f.transaction_number) { Warn "the job's transaction ($(Val $job.lrs_transaction_number)) differs from the file's ($($f.transaction_number))" }
  $ed = Get-Api "/api/v1/wf/jobs/$($job.id)/editing"
}

# --- 3. Digitising --------------------------------------------------------------------------------
Head '3. Digitising (Plot Parcel)'
$committed = @()
if ($ed) { $committed = @($posted | Where-Object { $_.job_id -eq $ed.cadastre_job_id }) }
if (-not $committed.Count) { $committed = $posted }
if ($job -and -not $job.is_open) {
  Info ("the job is closed ({0}): nothing left to digitise" -f $job.status_label)
} elseif ($ed) {
  if ($ed.area) { Ok ("job area {0:N1} m2 ({1})" -f [double]$ed.area_m2, (Val $ed.area_status)) } else { Todo 'no job area yet (Plot Parcel > Create Job Area, or Finish Sketch creates one)' }
  if ($ed.drafts -gt 0) {
    Ok "$($ed.drafts) draft parcel(s) in $($ed.reference)"
    $corners = AsList (Get-Api "/api/v1/wf/jobs/$($job.id)/editing/corners")
    Info ("{0} corner(s):" -f $corners.Count)
    foreach ($c in $corners) {
      $b = if ($c.beacon_number) { "$($c.beacon_number) (existing beacon)" } else { 'new: number typed at Commit' }
      Info ("  {0,2}  E {1,12:N3}  N {2,13:N3}  {3}" -f $c.point, [double]$c.easting, [double]$c.northing, $b)
    }
    SetNext 'Review the draft (Send To Task > Review Plotted Parcel), then Send To Task > Commit Parcel > Commit, typing every beacon number'
  } elseif (-not $committed.Count) {
    Todo 'no draft parcel yet'
    SetNext 'Plot Parcel > Construct Parcel > Digitize Parcel (Sketch): paste "BEACON EASTING NORTHING" one per line, Add pasted points, Finish Sketch, Save parcel'
  } else { Ok 'no drafts left (committed)' }
} else { Info 'waiting for the job' }

# --- 4. Commit ------------------------------------------------------------------------------------
Head '4. Commit (UPIN)'
if ($committed.Count) {
  foreach ($p in $committed) { Ok ("parcel {0} posted" -f $p.parcel_number) }
} else {
  Todo 'not committed: no posted parcel carries this file number yet'
  SetNext 'Commit Parcel > Commit (allocates the UPIN and creates the new beacons)'
}

# --- 5. Registry link -----------------------------------------------------------------------------
Head '5. Registry link (done in the Registry by the Registry admin)'
$linked = $false
foreach ($p in $committed) {
  try { $w = Get-Api ('/api/v1/registry/parcels/' + (Esc $p.parcel_number) + '/work') } catch { $w = $null }
  if ($w -and $w.linked) {
    $linked = $true
    Ok ("{0} linked: LRSID {1}, property {2}, transaction {3}" -f $p.parcel_number, (Val $w.su_id), (Val $w.property_number), (Val $w.transaction_number))
  } else { Todo "$($p.parcel_number) not linked yet" }
}
if ($committed.Count -and -not $linked -and (-not $job -or $job.is_open)) {
  SetNext ("Registry admin: open file $fileNo in the Registry, map > select UPIN $($committed[0].parcel_number) > Link. " +
           "The Registry map must show THIS server's parcels (GISServer / LWAMapURL = $Server/GISViewer/MapWorker.aspx). The link arrives here within a minute.")
}
if ($f.upin -and $committed.Count -and -not ($committed | Where-Object { $_.parcel_number -eq $f.upin })) {
  Warn "the Registry property holds UPIN $($f.upin), which is not this file's committed parcel"
}

# --- 6. Title deed plan ---------------------------------------------------------------------------
Head '6. Title deed plan (Generate Title Deed Plan)'
if ($ed) {
  if ($ed.cadastre_job_id) { Info ("site plan: {0}/api/v1/jobs/{1}/siteplan.pdf" -f $Server, $ed.cadastre_job_id) }
  else { Info 'site plan: opens once the job is worked on in the editor (Work On This Job)' }
  $sent = AsList (Get-Api "/api/v1/wf/jobs/$($job.id)/registry/sent")
  if ($sent.Count) { foreach ($s in $sent) { Ok ("plan sent to the Registry: {0}" -f (($s.PSObject.Properties | Where-Object { $_.Value -ne $null } | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join '  ')) } }
  elseif ($job -and -not $job.is_open) { Info 'no plan sent from this Cadastre (plans Aumentum sent are not recorded here)' }
  else {
    Todo 'no plan sent to the Registry'
    if ($linked -and $job.is_open) { SetNext 'Map: right-click the parcel > Start edit - title deed; check the site plan; Send plan (only if the plan should really be attached to the Registry transaction)' }
  }
} else { Info 'waiting for the job' }

# --- 7. Job completion ----------------------------------------------------------------------------
Head '7. Job completion'
if ($job) {
  switch ([int]$job.status) {
    3 { Ok 'completed: waiting for the Registry to approve or reject (LRS Commit)' }
    5 { Ok 'approved by the Registry' }
    7 { Warn 'rejected by the Registry' }
    9 { Warn 'cancelled' }
    default {
      if ($job.is_open) { Todo "open ($($job.status_label)) at task $($job.current_task)"; if ($linked) { SetNext 'Generate Title Deed Plan done: Complete Job' } }
      else { Info $job.status_label }
    }
  }
} else { Info 'waiting for the job' }

Write-Host ''
if ($flow.next) { Write-Host ("NEXT STEP: " + $flow.next) -ForegroundColor Yellow }
else { Write-Host 'Flow complete for this file.' -ForegroundColor Green }
