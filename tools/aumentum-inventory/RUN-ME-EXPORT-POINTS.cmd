@echo off
REM READ-ONLY export of Aumentum's survey points (the beacons, Plateau_State_Points) from
REM PRODUCTIONMC. Run inside the Remote Desktop session. Takes a few minutes.
REM Output: \\tsclient\inventory\aumentum-export\points.psv (back on the Linux machine).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-export.ps1" -OutDir "%~dp0." -Only points
echo.
echo Finished. You can close this window.
pause
