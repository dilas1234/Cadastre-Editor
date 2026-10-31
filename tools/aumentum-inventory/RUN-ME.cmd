@echo off
REM Double-click inside the Remote Desktop session. READ-ONLY: changes nothing on this server.
REM The report is saved next to this file (i.e. back on the Linux machine via \\tsclient).
echo Running read-only Aumentum inventory...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-inventory.ps1" -OutDir "%~dp0."
if errorlevel 1 (
  echo Could not save to the shared folder, saving to your Desktop instead...
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-inventory.ps1"
)
echo.
echo Finished. You can close this window and the Remote Desktop session.
pause
