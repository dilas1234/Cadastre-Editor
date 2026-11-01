@echo off
REM READ-ONLY export of the live parcels, parcel attributes (registry links), control points and
REM every issued UPIN from PRODUCTIONMC. Run as administrator inside the Remote Desktop session.
REM Output: \\tsclient\inventory\aumentum-export\ (back on the Linux machine).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-export.ps1" -OutDir "%~dp0."
echo.
echo Finished. You can close this window.
pause
