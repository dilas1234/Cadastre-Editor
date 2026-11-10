@echo off
REM AUMENTUM DATABASE CHECK on PRODUCTIONMC (10.10.10.6) - double-click (Run as administrator) inside
REM the Remote Desktop session. READ-ONLY: it changes nothing on this server. It finds which database
REM Aumentum's parcels are in, whether it answers, and what the logs say since Nov 5.
REM Output: \\tsclient\inventory\aumentum-capture\dbcheck-<host>-<time>.txt (back on the Linux machine).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-dbcheck.ps1" -OutDir "%~dp0."
echo.
echo Finished. Lines in red are the problems. Tell Claude the report is ready.
pause
