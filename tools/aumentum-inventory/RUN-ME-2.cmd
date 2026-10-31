@echo off
REM PASS 2 - double-click inside the Remote Desktop session. READ-ONLY: changes nothing on this server.
echo Running read-only Aumentum deep inventory (pass 2)...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-deep.ps1" -OutDir "%~dp0."
if errorlevel 1 (
  echo Could not save to the shared folder, saving to your Desktop instead...
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-deep.ps1"
)
echo.
echo Finished. You can close this window.
pause
