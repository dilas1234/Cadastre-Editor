@echo off
REM Copies back the Registry export that the last RUN-ME-REGISTRY-EXPORT.cmd left on the Registry
REM server (it stopped while copying). Does NOT export again. Then does the PRODUCTIONMC part.
REM Run on PRODUCTIONMC as administrator, inside the Remote Desktop session. READ-ONLY.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-export-remote.ps1" -OutDir "%~dp0." -Resume
echo.
echo Finished. You can close this window.
pause
