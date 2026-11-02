@echo off
REM Registry simulator: calls the new Cadastre (10.10.10.9) the way the Registry will, without
REM changing any Registry setting. Run on PRODUCTIONMC inside the Remote Desktop session.
REM READ-ONLY. Asks for: a test UPIN, a transaction number, the job-service account, a Registry user.
REM Add -WriteTest after the .ps1 path to also send the Link message (re-sends current values only).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-registry-simulate.ps1" -OutDir "%~dp0."
echo.
echo Finished. You can close this window.
pause
