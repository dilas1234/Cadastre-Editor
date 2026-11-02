@echo off
REM The Registry desktop client libraries only (its Properties page map / linking), from registry.plagis.com.
REM Run on PRODUCTIONMC as administrator, inside the Remote Desktop session. READ-ONLY. About a minute.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-map-capture.ps1" -OutDir "%~dp0." -RegistryOnly
echo.
echo Finished. You can close this window.
pause
