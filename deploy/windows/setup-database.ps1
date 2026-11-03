<#
  Creates the Cadastre database in the local PostgreSQL 14 (port 5433 on 10.10.10.4; 5432 there is
  PostgreSQL 9.2, too old). Run once, elevated, BEFORE install-cadastre.ps1:

    powershell -ExecutionPolicy Bypass -File .\setup-database.ps1

  Asks for the PostgreSQL "postgres" password. Creates (or resets the password of) role "cadastre",
  database "cadastre" owned by it, and the postgis + uuid-ossp extensions. Writes the connection
  string, with a generated password, into $InstallDir\cadastre.env. Safe to re-run.
#>
param(
  [string]$PgBin = 'C:\Program Files\PostgreSQL\14\bin',
  [int]$Port = 5433,
  [string]$InstallDir = 'C:\Cadastre'
)
$ErrorActionPreference = 'Stop'
$kit = Split-Path -Parent $MyInvocation.MyCommand.Path
$psql = Join-Path $PgBin 'psql.exe'
if (-not (Test-Path $psql)) { throw "psql not found at $psql (use -PgBin)" }

$sec = Read-Host -AsSecureString "Password of PostgreSQL user 'postgres' (port $Port)"
$env:PGPASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
  [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec))

function Invoke-Psql([string]$db, [string]$sql) {
  # Via a file: PowerShell 4 mangles double quotes in native-command arguments. Under 'Continue':
  # with 'Stop', a NOTICE on stderr would throw.
  $f = [IO.Path]::GetTempFileName()
  Set-Content -Path $f -Value $sql -Encoding ASCII
  $ErrorActionPreference = 'Continue'
  $out = & $psql -h localhost -p $Port -U postgres -d $db -v ON_ERROR_STOP=1 -q -tA -f $f 2>&1 |
    Where-Object { "$_" -notmatch '^(psql:.*)?NOTICE:' }
  $code = $LASTEXITCODE
  Remove-Item $f
  if ($code -ne 0) { throw "psql failed: $out" }
  return ("$out").Trim()
}

Write-Host (Invoke-Psql 'postgres' 'select version()')

# Alphanumeric only, so it needs no escaping in SQL or in the connection URL.
$chars = [char[]]'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
$rng = New-Object Security.Cryptography.RNGCryptoServiceProvider
$bytes = New-Object byte[] 24
$rng.GetBytes($bytes)
$pw = -join ($bytes | ForEach-Object { $chars[$_ % $chars.Length] })

if ((Invoke-Psql 'postgres' "select 1 from pg_roles where rolname = 'cadastre'") -eq '1') {
  Invoke-Psql 'postgres' "alter role cadastre with login password '$pw'" | Out-Null
  Write-Host "Role cadastre existed: password reset."
} else {
  Invoke-Psql 'postgres' "create role cadastre login password '$pw'" | Out-Null
  Write-Host "Role cadastre created."
}
if ((Invoke-Psql 'postgres' "select 1 from pg_database where datname = 'cadastre'") -ne '1') {
  Invoke-Psql 'postgres' 'create database cadastre owner cadastre' | Out-Null
  Write-Host "Database cadastre created."
}
# Extensions need a superuser; the server's migrations then find them present.
Invoke-Psql 'cadastre' 'create extension if not exists postgis' | Out-Null
Invoke-Psql 'cadastre' 'create extension if not exists "uuid-ossp"' | Out-Null
Write-Host ("PostGIS " + (Invoke-Psql 'cadastre' 'select postgis_lib_version()'))
Remove-Item Env:\PGPASSWORD

# Write the connection string into cadastre.env.
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$envFile = Join-Path $InstallDir 'cadastre.env'
if (-not (Test-Path $envFile)) { Copy-Item (Join-Path $kit 'cadastre.env.example') $envFile }
$url = "CADASTRE_DATABASE_URL=postgres://cadastre:$pw@localhost:$Port/cadastre"
$lines = Get-Content $envFile | ForEach-Object { if ($_ -match '^\s*CADASTRE_DATABASE_URL\s*=') { $url } else { $_ } }
Set-Content -Path $envFile -Value $lines -Encoding ASCII
& icacls $envFile /inheritance:r /grant:r 'Administrators:F' 'SYSTEM:F' | Out-Null
Write-Host "Wrote CADASTRE_DATABASE_URL to $envFile. Next: .\install-cadastre.ps1"
