<#
  Registry simulator: calls the new Cadastre (default http://10.10.10.9:8080) exactly as the
  Aumentum Registry would once its settings point there, WITHOUT changing any Registry setting.
  Run on PRODUCTIONMC inside the Remote Desktop session (RUN-ME-REGISTRY-SIMULATE.cmd).

  1. Network: can PRODUCTIONMC, and the Registry server (over remote management), reach the Cadastre?
  2. Job service (Registry setting mcServiceAddress): Authenticate as the Registry's mcUserName with
     its password, then read-only calls: GetAllJobs, GetConfiguredRegions, GetParcelAttributesByUPIN,
     GetJobStatusByTransaction, LogOut.
  3. Registry ticket: signs in to the Registry API (AuthenticationService.startSession) and checks
     that the Cadastre's map accepts the ticket (it asks the Registry isTicketValid).
  4. The Registry's map (settings GISServer / LWAMapURL) in Internet Explorer, the browser the Registry
     desktop embeds: the same POST as its MapWorkerControl, wait for "Map Ready", you select parcels,
     then doAction("SelectionSet","GetSelectedObjects"), doAction("Map","GetExtentAsString"),
     doAction("Editor","IsEditInProgress"), decoded as the Registry decodes them.
  5. Only with -WriteTest: UpdateCadastralData (what the Registry sends on Link) for the selected
     parcels, re-sending the values they already hold, so nothing changes but the edit date.

  READ-ONLY by default. Never writes to the Registry. Passwords and tickets are never printed or saved.
  PowerShell 4 compatible.
#>
param(
  [string]$Cadastre = 'http://10.10.10.9:8080',
  [string]$Registry = 'http://10.10.10.5:8080/lrs/api/',
  [string]$RegistryHost = 'registry.plagis.com',
  [string]$Upin = '',
  [string]$Transaction = '',
  [string]$OutDir = $PSScriptRoot,
  [switch]$SkipRegistryServer,
  [switch]$SkipMap,
  [switch]$WriteTest
)
$ErrorActionPreference = 'Stop'
$AUTH = 'http://www.alfresco.org/ws/service/authentication/1.0'
$CNS = 'http://www.landsystems.com/multicadastre/in4/service'
$Cadastre = $Cadastre.TrimEnd('/')
if (-not $Registry.EndsWith('/')) { $Registry += '/' }
$jobUrl = "$Cadastre/ApplicationServerAdmin/WebServices/LRSWebServices.asmx"
$mapUrl = "$Cadastre/GISViewer/MapWorker.aspx?ct=indexer&justmap=1&map=property-select&lang=en-US"
$svcUrl = "$Cadastre/GISViewer/WebServices/CadastralWebService.asmx"
$report = New-Object Collections.Generic.List[string]
$results = New-Object Collections.Generic.List[object]
function Say([string]$s, [string]$c = 'Gray') { Write-Host $s -ForegroundColor $c; $report.Add($s) }
function Result([string]$name, [bool]$ok, [string]$detail) {
  $results.Add((New-Object PSObject -Property @{ Step = $name; Ok = $ok; Detail = $detail }))
  if ($ok) { Say ("  PASS  {0}: {1}" -f $name, $detail) 'Green' } else { Say ("  FAIL  {0}: {1}" -f $name, $detail) 'Red' }
}
function Esc([string]$s) { [Security.SecurityElement]::Escape($s) }
function Tag([string]$xml, [string]$name) {
  $m = [regex]::Match($xml, "<(?:\w+:)?$name(?:\s[^>]*)?>([^<]*)</(?:\w+:)?$name>")
  if ($m.Success) { [Net.WebUtility]::HtmlDecode($m.Groups[1].Value) } else { $null }
}
function Http([string]$Url, [string]$Method = 'GET', [string]$Body = $null, [string]$ContentType = 'text/xml; charset=utf-8', [hashtable]$Headers = @{}, $Cookies = $null, [int]$TimeoutMs = 60000) {
  $req = [Net.HttpWebRequest]::Create($Url)
  $req.Method = $Method; $req.Timeout = $TimeoutMs; $req.Proxy = $null
  if ($Cookies) { $req.CookieContainer = $Cookies }
  foreach ($k in $Headers.Keys) { $req.Headers.Add($k, $Headers[$k]) }
  if ($Method -eq 'POST') {
    $bytes = [Text.Encoding]::UTF8.GetBytes([string]$Body)
    $req.ContentType = $ContentType; $req.ContentLength = $bytes.Length
    $st = $req.GetRequestStream(); $st.Write($bytes, 0, $bytes.Length); $st.Close()
  }
  $resp = $null
  try { $resp = $req.GetResponse() } catch [Net.WebException] { $resp = $_.Exception.Response; if (-not $resp) { throw } }
  $rd = New-Object IO.StreamReader($resp.GetResponseStream())
  $txt = $rd.ReadToEnd(); $code = [int]$resp.StatusCode; $resp.Close()
  New-Object PSObject -Property @{ Code = $code; Text = $txt }
}
function JobCall([string]$op, [string]$inner, $cookies) {
  $env = "<?xml version=`"1.0`" encoding=`"utf-8`"?><soap:Envelope xmlns:soap=`"http://schemas.xmlsoap.org/soap/envelope/`"><soap:Body><$op xmlns=`"http://tempuri.org/`">$inner</$op></soap:Body></soap:Envelope>"
  Http $jobUrl 'POST' $env 'text/xml; charset=utf-8' @{ SOAPAction = "`"http://tempuri.org/$op`"" } $cookies
}
function Decode([string]$s) {
  if (-not $s) { return $null }
  $b = $s -replace '^.*?\[BEGIN\]', '' -replace '^<base64Binary[^>]*>', '' -replace '\[END\]\s*$', '' -replace '</base64Binary>\s*$', ''
  [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($b.Trim())) | ConvertFrom-Json
}
function Encode([string]$json) { '[BEGIN]' + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json)) + '[END]' }

Say "Registry simulator on $env:COMPUTERNAME, $(Get-Date -Format s)" 'Cyan'
Say "Cadastre: $Cadastre   Registry API: $Registry   (read-only$(if ($WriteTest) { ', with -WriteTest' }))"
if (-not $Upin) { $Upin = (Read-Host 'A UPIN to test with, e.g. PL000000022581 (Enter to skip parcel checks)').Trim() }
if (-not $Transaction) { $Transaction = (Read-Host 'A transaction number, e.g. PLS000138648 (Enter to skip)').Trim() }

# ---------------------------------------------------------------- 1. Network
Say "`n[1] Network" 'Cyan'
try { $r = Http "$jobUrl`?WSDL"; Result 'PRODUCTIONMC -> Cadastre' ($r.Code -eq 200 -and $r.Text -match 'definitions') "HTTP $($r.Code)" }
catch { Result 'PRODUCTIONMC -> Cadastre' $false $_.Exception.Message }
if (-not $SkipRegistryServer) {
  try {
    $s = $null
    try { $s = New-PSSession -ComputerName $RegistryHost }
    catch { Write-Host "Sign-in refused on $RegistryHost; enter an administrator account of the Registry server." -ForegroundColor Yellow; $s = New-PSSession -ComputerName $RegistryHost -Credential (Get-Credential) }
    try {
      $out = Invoke-Command -Session $s -ArgumentList "$jobUrl`?WSDL" -ScriptBlock {
        param($u)
        try { $wc = New-Object Net.WebClient; $wc.Proxy = $null; $t = $wc.DownloadString($u); 'OK ' + $t.Length } catch { 'FAIL ' + $_.Exception.Message }
      }
      Result 'Registry server -> Cadastre' ($out -like 'OK*') "$out (the Registry's job-service calls come from here)"
    } finally { Remove-PSSession $s }
  } catch { Result 'Registry server -> Cadastre' $false "could not run on ${RegistryHost}: $($_.Exception.Message)" }
}

# ---------------------------------------------------------------- 2. Job service
Say "`n[2] Job service (mcServiceAddress), as the Registry's Java client" 'Cyan'
$mc = Get-Credential -UserName 'administrator' -Message 'The Registry''s job-service account (multicadastre mcUserName / mcPassword = CADASTRE_MC_SERVICE_USER / _PASSWORD)'
$cookies = New-Object Net.CookieContainer
try {
  $r = JobCall 'Authenticate' ("<user>{0}</user><password>{1}</password>" -f (Esc $mc.UserName), (Esc $mc.GetNetworkCredential().Password)) $cookies
  $ok = (Tag $r.Text 'AuthenticateResult') -eq 'true'
  Result 'Authenticate' $ok ("as {0}; session cookie {1}" -f $mc.UserName, $(if ($cookies.GetCookies([Uri]$jobUrl).Count) { 'set' } else { 'missing' }))
  if ($ok) {
    $r = JobCall 'GetAllJobs' '' $cookies
    $names = [regex]::Matches($r.Text, '<Name>([^<]*)</Name>') | ForEach-Object { $_.Groups[1].Value }
    Result 'GetAllJobs' ($r.Code -eq 200 -and @($names).Count -gt 0) ("{0} job type(s), e.g. {1}" -f @($names).Count, ((@($names) | Select-Object -First 4) -join '; '))
    $r = JobCall 'GetConfiguredRegions' '' $cookies
    $region = Tag $r.Text 'string'; if (-not $region) { $region = 'Plateau_State' }
    Result 'GetConfiguredRegions' ($r.Code -eq 200) "region $region"
    if ($Upin) {
      $r = JobCall 'GetParcelAttributesByUPIN' ("<regionName>{0}</regionName><upin>{1}</upin>" -f (Esc $region), (Esc $Upin)) $cookies
      $fields = [regex]::Matches($r.Text, '<FieldName>([^<]*)</FieldName>\s*<FieldValue>([^<]*)</FieldValue>') | Where-Object { $_.Groups[1].Value -match '^(UPIN|LRSID|LinkageStatus|PropertyNumber|OBJECTID)$' } | ForEach-Object { $_.Groups[1].Value + '=' + $_.Groups[2].Value }
      Result 'GetParcelAttributesByUPIN' ($r.Code -eq 200 -and $r.Text -notmatch 'faultstring') $(if (@($fields).Count) { @($fields) -join ', ' } else { 'answered (no attribute record found for this UPIN)' })
    }
    if ($Transaction) {
      $digits = ($Transaction -replace '\D', '').TrimStart('0'); if (-not $digits) { $digits = '0' }
      $r = JobCall 'GetJobStatusByTransaction' "<transactionNumber>$digits</transactionNumber>" $cookies
      $jobs = [regex]::Matches($r.Text, '<WFJob>').Count
      Result 'GetJobStatusByTransaction' ($r.Code -eq 200 -and $r.Text -notmatch 'faultstring') "$jobs job(s) for $Transaction (the contract passes the number as an int: $digits)"
    }
    $r = JobCall 'LogOut' '' $cookies
    Result 'LogOut' ($r.Code -eq 200) "HTTP $($r.Code)"
  }
} catch { Result 'Job service' $false $_.Exception.Message }

# ---------------------------------------------------------------- 3. Registry ticket
Say "`n[3] Registry ticket (what the Registry desktop hands the map)" 'Cyan'
$ticket = $null; $regUser = $null
try {
  $rc = Get-Credential -UserName 'admin' -Message 'A Registry user (as on the Registry desktop; REGISTRY_USER / REGISTRY_PASSWORD in .env)'
  $regUser = $rc.UserName
  $body = "<?xml version=`"1.0`" encoding=`"utf-8`"?><soap:Envelope xmlns:soap=`"http://schemas.xmlsoap.org/soap/envelope/`"><soap:Header/><soap:Body><auth:startSession xmlns:auth=`"$AUTH`"><auth:username>{0}</auth:username><auth:password>{1}</auth:password></auth:startSession></soap:Body></soap:Envelope>" -f (Esc $rc.UserName), (Esc $rc.GetNetworkCredential().Password)
  $r = Http ($Registry + 'AuthenticationService') 'POST' $body 'text/xml; charset=utf-8' @{ SOAPAction = "`"$AUTH/startSession`"" }
  $ticket = Tag $r.Text 'ticket'
  Result 'Registry sign-in' ([bool]$ticket) $(if ($ticket) { "ticket received for $regUser (not shown)" } else { "HTTP $($r.Code), no ticket" })
  if ($ticket) {
    $form = 'auth=1&lrsuser=' + [Uri]::EscapeDataString($regUser) + '&lrsticket=' + [Uri]::EscapeDataString($ticket)
    $r = Http $mapUrl 'POST' $form 'application/x-www-form-urlencoded'
    Result 'Cadastre accepts the ticket' ($r.Text -match 'silverlightControl') $(if ($r.Text -match 'silverlightControl') { 'map page served (the Cadastre checked the ticket with the Registry)' } else { ([regex]::Match($r.Text, '<p>([^<]*)</p>').Groups[1].Value) })
    $r = Http $mapUrl 'POST' 'auth=1&lrsuser=nobody&lrsticket=not-a-ticket' 'application/x-www-form-urlencoded'
    Result 'Cadastre refuses a bad ticket' ($r.Text -notmatch 'silverlightControl') 'refusal page'
  }
} catch { Result 'Registry ticket' $false $_.Exception.Message }

# ---------------------------------------------------------------- 4. The map in Internet Explorer
$selected = @()
if (-not $SkipMap -and $ticket) {
  Say "`n[4] The Registry's map in Internet Explorer (MapWorkerControl's browser)" 'Cyan'
  $ie = $null
  try {
    $inputJson = '[]'
    if ($Upin) {
      $inputJson = '[{"LayerName":"Plateau_State_Parcels","IsReadOnly":false,"IsSelectionSet":true,"IsEditPredefinedValues":false,"SearchType":"AND",' +
               '"Fields":[{"Name":"UPIN","Alias":"UPIN","IsLayerField":true,"IsOnlySearchCriteria":false,"IsPrimary":true}],' +
               '"Entities":[{"UUID":0,"IsOnlySearchCriteria":false,"Attributes":[{"Name":"UPIN","Value":"' + $Upin + '"}]}]}]'
    }
    $form = 'auth=1&lrsuser=' + [Uri]::EscapeDataString($regUser) + '&lrsticket=' + [Uri]::EscapeDataString($ticket) + '&initparam_inputobjects=' + [Uri]::EscapeDataString((Encode $inputJson))
    $ie = New-Object -ComObject InternetExplorer.Application
    $ie.Visible = $true
    $ie.Navigate2($mapUrl, 0, '', [Text.Encoding]::ASCII.GetBytes($form), "Content-Type: application/x-www-form-urlencoded`r`n")
    function Doc { $ie.Document }
    function El([string]$id) { $d = Doc; $e = $null; try { $e = $d.getElementById($id) } catch {}; if (-not $e) { try { $e = $d.IHTMLDocument3_getElementById($id) } catch {} }; $e }
    function Js([string]$code) {
      $d = Doc; $w = $null
      try { $w = $d.parentWindow } catch {}
      if (-not $w) { $w = [__ComObject].InvokeMember('parentWindow', [Reflection.BindingFlags]::GetProperty, $null, $d, $null) }
      try { $w.execScript($code, 'JavaScript') | Out-Null } catch { [__ComObject].InvokeMember('execScript', [Reflection.BindingFlags]::InvokeMethod, $null, $w, @($code, 'JavaScript')) | Out-Null }
    }
    function Ask([string]$expr) {
      Js "var o=document.getElementById('simOut');if(!o){o=document.createElement('input');o.type='hidden';o.id='simOut';document.body.appendChild(o);}o.value=String($expr);"
      (El 'simOut').value
    }
    $ready = $false; $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 90) {
      Start-Sleep -Milliseconds 500
      $ev = $null; try { $ev = El 'indexerEvents' } catch {}
      if ($ev -and $ev.value -eq 'Map Ready') { $ready = $true; break }
    }
    $hasPage = [bool](El 'silverlightControl')
    Result 'Map page in IE' $hasPage $(if ($hasPage) { "silverlightControl present ($($ie.LocationURL))" } else { 'the page did not load: ' + $ie.LocationURL })
    Result 'Map Ready' $ready $(if ($ready) { 'indexerEvents = "Map Ready"' } else { 'not within 90 s (details below)' })
    # What IE made of the page: its document mode, the libraries, script errors, load state.
    $diag = @()
    try { $diag += 'document mode ' + (Doc).documentMode } catch {}
    try { $diag += 'page state ' + (Doc).readyState } catch {}
    foreach ($q in @('typeof L', 'typeof proj4', 'typeof doAction')) { try { $diag += $q + ' = ' + (Ask $q) } catch { $diag += "$q unreadable" } }
    try { $errs = (El 'mapErrors').value; $diag += 'script errors: ' + $(if ($errs) { $errs } else { 'none' }) } catch {}
    try { $r = Http 'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer?f=json' 'GET' $null 'text/xml' @{} $null 10000; $diag += "internet imagery reachable from this server: HTTP $($r.Code)" } catch { $diag += 'internet imagery NOT reachable from this server (the map still works, without the photo background)' }
    Say ('        ' + ($diag -join "`n        "))
    if ($ready) {
      Write-Host "`nThe map is open in Internet Explorer. Select the parcel(s) as the Registry admin would" -ForegroundColor Yellow
      Write-Host "(the UPIN's parcel is pre-selected; click to add or remove). Then come back here and press Enter." -ForegroundColor Yellow
      Read-Host | Out-Null
      $raw = Ask 'doAction("SelectionSet","GetSelectedObjects")'
      $groups = Decode $raw
      foreach ($g in @($groups)) { foreach ($en in @($g.Entities)) {
        $a = @{}; foreach ($x in @($en.Attributes)) { $a[$x.Name] = $x.Value }
        $selected += New-Object PSObject -Property @{ ObjectID = $en.ObjectID; InputUUID = $en.InputUUID; Layer = $g.LayerName; Attr = $a }
      } }
      Result 'GetSelectedObjects' ($raw -like '`[BEGIN`]*' -and $selected.Count -gt 0) ("{0} parcel(s): {1}" -f $selected.Count, (($selected | ForEach-Object { "OBJECTID $($_.ObjectID) UPIN $($_.Attr['UPIN']) for property $($_.InputUUID)" }) -join '; '))
      $ext = Decode (Ask 'doAction("Map","GetExtentAsString")')
      Result 'GetExtentAsString' ($ext -and $ext.WKID -eq 32632) $(if ($ext) { "WKID $($ext.WKID): {0:N1}, {1:N1} - {2:N1}, {3:N1}" -f $ext.MinX, $ext.MinY, $ext.MaxX, $ext.MaxY } else { 'no answer' })
      $edit = Ask 'doAction("Editor","IsEditInProgress")'
      Result 'IsEditInProgress' ($edit -eq 'false') $edit
    }
  } catch { Result 'Map in IE' $false $_.Exception.Message }
  finally { if ($ie) { try { $ie.Quit() } catch {} } }
}

# ---------------------------------------------------------------- 5. Link message (opt-in)
if ($WriteTest -and $ticket -and $selected.Count) {
  Say "`n[5] UpdateCadastralData (the Registry's Link), re-sending the parcels' current values" 'Cyan'
  foreach ($p in $selected) {
    $vals = ''
    foreach ($f in 'LRSID', 'LinkageStatus', 'PropertyNumber') {
      if ($p.Attr.ContainsKey($f)) { $vals += "<UpdateValue><Field>$f</Field><Value xsi:type=`"xsd:string`">$(Esc $p.Attr[$f])</Value></UpdateValue>" }
    }
    if (-not $vals) { Result "Link message for OBJECTID $($p.ObjectID)" $true 'skipped: the parcel holds no link values to re-send'; continue }
    $env = "<?xml version=`"1.0`" encoding=`"utf-8`"?><soap:Envelope xmlns:soap=`"http://schemas.xmlsoap.org/soap/envelope/`" xmlns:xsi=`"http://www.w3.org/2001/XMLSchema-instance`" xmlns:xsd=`"http://www.w3.org/2001/XMLSchema`">" +
           "<soap:Header><SecurityHeader xmlns=`"$CNS`"><User>$(Esc $regUser)</User><Password>$(Esc $ticket)</Password></SecurityHeader></soap:Header>" +
           "<soap:Body><UpdateCadastralData xmlns=`"$CNS`"><updateList><UpdateDataList><LayerNameOrAlias>sde.SDE.Plateau_State_Parcels</LayerNameOrAlias><Records>" +
           "<UpdateRecord xsi:type=`"UpdateRecordByOID`"><Values>$vals</Values><ObjectID>$($p.ObjectID)</ObjectID></UpdateRecord></Records></UpdateDataList></updateList></UpdateCadastralData></soap:Body></soap:Envelope>"
    try {
      $r = Http $svcUrl 'POST' $env 'text/xml; charset=utf-8' @{ SOAPAction = "`"$CNS/UpdateCadastralData`"" }
      $err = Tag $r.Text 'Error'
      Result "Link message for OBJECTID $($p.ObjectID)" ($r.Code -eq 200 -and -not $err) $(if ($err) { $err } else { 'accepted (values unchanged; audit entry registry_map_update)' })
    } catch { Result "Link message for OBJECTID $($p.ObjectID)" $false $_.Exception.Message }
  }
}

# ---------------------------------------------------------------- end
if ($ticket) {
  try { Http ($Registry + 'AuthenticationService') 'POST' ("<?xml version=`"1.0`" encoding=`"utf-8`"?><soap:Envelope xmlns:soap=`"http://schemas.xmlsoap.org/soap/envelope/`"><soap:Header/><soap:Body><auth:endSession xmlns:auth=`"$AUTH`"><auth:ticket>{0}</auth:ticket></auth:endSession></soap:Body></soap:Envelope>" -f (Esc $ticket)) 'text/xml; charset=utf-8' @{ SOAPAction = "`"$AUTH/endSession`"" } | Out-Null } catch {}
}
$fail = @($results | Where-Object { -not $_.Ok }).Count
Say ("`nSummary: {0} passed, {1} failed." -f ($results.Count - $fail), $fail) $(if ($fail) { 'Yellow' } else { 'Green' })
$dir = Join-Path $OutDir 'lrs-export\simulator'
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$file = Join-Path $dir ("simulate-{0}.txt" -f (Get-Date -Format 'yyyyMMdd-HHmm'))
[IO.File]::WriteAllLines($file, [string[]]$report, (New-Object Text.UTF8Encoding($false)))
Write-Host "Report: $file" -ForegroundColor Green
