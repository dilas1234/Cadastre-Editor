@echo off
REM LRS JBOSS LOG CHECK on the REGISTRY server. READ-ONLY: it changes nothing, safe while LRSAS runs.
REM Copy this file and lrs-logcheck.ps1 to C:\Temp\logcheck first, then Run as administrator there.
REM Output: lrs-capture\logcheck-<host>-<time>.txt next to this file.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-logcheck.ps1" -OutDir "%~dp0."
echo.
echo Finished. Copy lrs-capture\logcheck-*.txt to \\tsclient\inventory\lrs-capture and tell Claude.
pause
