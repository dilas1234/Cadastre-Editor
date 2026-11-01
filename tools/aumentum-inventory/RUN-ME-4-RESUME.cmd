@echo off
REM Resume PASS 4 on PRODUCTIONMC from section 4, skipping the copies and the registry-settings
REM tree already captured. READ-ONLY (SELECT statements only). Close the slow pass 4 window first.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-pass4.ps1" -OutDir "%~dp0." -From 4
echo.
echo Finished. You can close this window.
pause
