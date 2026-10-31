@echo off
REM PASS 3 - double-click (Run as administrator) inside the Remote Desktop session. READ-ONLY.
echo Running read-only Aumentum pass 3 (config files, PDF guides, database metadata)...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-pass3.ps1" -OutDir "%~dp0."
if errorlevel 1 (
  echo Could not save to the shared folder, saving to your Desktop instead...
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-pass3.ps1"
)
echo.
echo Finished. You can close this window.
pause
