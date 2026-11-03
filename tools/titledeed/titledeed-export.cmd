@echo off
REM Export the title deed plan workflow (Deed Plan Creationv2): definition, jobs, task history.
REM Read-only. Output: a titledeed-workflow-<server>-<time> folder next to this script.
REM   titledeed-export.cmd                              (server http://10.10.10.9:8080)
REM   titledeed-export.cmd http://10.10.10.7:8080
REM   titledeed-export.cmd http://10.10.10.9:8080 -AllHistory   (task history of every job: slow)
set "SERVER=%~1"
if "%SERVER%"=="" set "SERVER=http://10.10.10.9:8080"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0titledeed-export.ps1" -Server "%SERVER%" -OutDir "%~dp0." %2 %3
echo.
pause
