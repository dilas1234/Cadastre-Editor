@echo off
REM Phase 5 (Registry linkage) capture. Run this on PRODUCTIONMC (inside the Remote Desktop session),
REM as administrator. READ-ONLY on both machines.
REM  - Registry server registry.plagis.com (10.10.10.5), over remote management: the LRS43 structure,
REM    the linkage rows WITHOUT personal data, and the LRS application's model definitions.
REM  - This machine: the Registry extension of the Cadastre desktop, its COM commands, and the
REM    GISViewer the Registry calls back.
REM Results come back next to this file in lrs-export\ (\\tsclient\inventory). About 10-30 minutes.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-export-remote.ps1" -OutDir "%~dp0."
echo.
echo Finished. You can close this window.
pause
