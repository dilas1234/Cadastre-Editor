@echo off
REM Title deed plan flow for one Registry file, read-only.
REM   titledeed-flow.cmd PL67363
REM   titledeed-flow.cmd PL67363 http://10.10.10.7:8080
REM The server defaults to the test server http://10.10.10.9:8080.
if "%~1"=="" (
  echo Usage: %~nx0 ^<file no. PL... or transaction PLS...^> [server, default http://10.10.10.9:8080]
  exit /b 1
)
if "%~2"=="" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0titledeed-flow.ps1" -File "%~1"
) else (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0titledeed-flow.ps1" -File "%~1" -Server "%~2"
)
