@echo off
REM Run this on PRODUCTIONMC (inside the Remote Desktop session), as administrator.
REM It reaches the Registry server registry.plagis.com (10.10.10.5) over remote management and runs
REM the READ-ONLY Registry inventory there. Results come back next to this file (\\tsclient\inventory).
REM You can run it while pass 4 is still running.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-inventory-remote.ps1" -OutDir "%~dp0."
echo.
echo Finished. You can close this window.
pause
