@echo off
REM PASS 3 + PASS 4 on PRODUCTIONMC - double-click (Run as administrator) inside the Remote Desktop
REM session. READ-ONLY: it copies files and runs SELECT statements; nothing on this server changes.
REM Results land next to this file (back on the Linux machine via \\tsclient\inventory).
echo Pass 3: config files, PDF guides, database metadata...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-pass3.ps1" -OutDir "%~dp0."
echo.
echo Pass 4: templates, sample plans, link fields, highest UPIN, workflow tables...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-pass4.ps1" -OutDir "%~dp0."
if errorlevel 1 (
  echo Could not save to the shared folder, saving to your Desktop instead...
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-pass4.ps1"
)
echo.
echo Finished. You can close this window.
pause
