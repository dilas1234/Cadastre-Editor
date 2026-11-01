@echo off
REM Registry DATABASE only (sections 11-13) - run as administrator on PRODUCTIONMC. READ-ONLY.
REM Uses the SQL login in lrs-sql.env; everything else was captured by the full run already.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-inventory-remote.ps1" -OutDir "%~dp0." -From 11
echo.
echo Finished. You can close this window.
pause
