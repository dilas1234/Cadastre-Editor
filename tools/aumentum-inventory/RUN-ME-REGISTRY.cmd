@echo off
REM REGISTRY (LRS43, 10.10.10.5) - run as administrator ON 10.10.10.5. READ-ONLY: it reads files,
REM fetches the SOAP service descriptions (?wsdl) from this machine and runs SELECT statements.
REM Nothing on this server changes. Results are saved next to this file, or on the Desktop.
echo Running read-only Registry (LRS) inventory...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-inventory.ps1" -OutDir "%~dp0."
if errorlevel 1 (
  echo Could not save next to this file, saving to your Desktop instead...
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-inventory.ps1"
)
echo.
echo Finished. Send back the lrs-inventory-*.txt file and the lrs-capture folder.
pause
