@echo off
REM Phase 5: captures the Registry map (GISViewer Silverlight client), the Registry job service
REM code (WebAdmin / ApplicationServerAdmin) and the Registry desktop client map control.
REM Run on PRODUCTIONMC as administrator, inside the Remote Desktop session. READ-ONLY. A few minutes.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-map-capture.ps1" -OutDir "%~dp0."
echo.
echo Finished. You can close this window.
pause
