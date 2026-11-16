@echo off
REM REGISTRY TRANSACTION STATUS on REGISTRY (10.10.10.5). READ-ONLY: SELECT statements only, changes nothing.
REM Default: the 4 files psatdom worked on 10-12 Nov 2026. Other files:
REM   RUN-ME-TRANSACTION-STATUS.cmd "10000000143500,10000000143503"   (keep the quotes: cmd splits on commas)
REM Output: lrs-capture\transaction-status-<time>.txt next to this file.
if "%~1"=="" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-transaction-status.ps1" -OutDir "%~dp0."
) else (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-transaction-status.ps1" -Ids "%~1" -OutDir "%~dp0."
)
echo.
echo Finished. If you ran this from C:\Temp, copy lrs-capture\transaction-status-*.txt to \\tsclient\inventory\lrs-capture and tell Claude.
pause
