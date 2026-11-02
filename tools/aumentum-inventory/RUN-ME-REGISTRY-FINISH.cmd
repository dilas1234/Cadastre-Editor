@echo off
REM Finishes the Registry export: confirms every Registry file is here (one quick folder scan),
REM removes the export temp folder on the Registry server, and captures the GISViewer on this machine.
REM Does NOT export again and does not redo the parts already captured. READ-ONLY.
REM Run on PRODUCTIONMC as administrator, inside the Remote Desktop session. A few minutes.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-export-remote.ps1" -OutDir "%~dp0." -Resume -Parts gisviewer
echo.
echo Finished. You can close this window.
pause
