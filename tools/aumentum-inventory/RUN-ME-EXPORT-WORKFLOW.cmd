@echo off
REM READ-ONLY export of Aumentum's application database (MCServer) on PRODUCTIONMC: workflow
REM definitions, all job and task instances with their Registry links, users/groups, the UPIN
REM register, lineage, locks and the event log. No passwords; binary values as sizes only.
REM Run as administrator inside the Remote Desktop session. Output: aumentum-export\mcserver\
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-export-mcserver.ps1" -OutDir "%~dp0."
echo.
echo Finished. You can close this window.
pause
