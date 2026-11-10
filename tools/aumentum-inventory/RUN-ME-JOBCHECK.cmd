@echo off
REM AUMENTUM JOB SERVICE CHECK on PRODUCTIONMC (10.10.10.6). READ-ONLY: it changes nothing.
REM Copy this file and aumentum-jobcheck.ps1 to C:\Temp\dbcheck first, then Run as administrator there.
REM Output: aumentum-capture\jobcheck-<host>-<time>.txt next to this file.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-jobcheck.ps1" -OutDir "%~dp0."
echo.
echo Finished. Copy aumentum-capture\jobcheck-*.txt to \\tsclient\inventory\aumentum-capture and tell Claude.
pause
