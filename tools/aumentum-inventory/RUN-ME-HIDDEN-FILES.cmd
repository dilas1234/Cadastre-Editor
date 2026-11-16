@echo off
REM REGISTRY HIDDEN-FILES CHECKLIST on REGISTRY (10.10.10.5). READ-ONLY: SELECT statements only, changes nothing.
REM Safe to run while LRSAS is rebuilding its index. Lists the files the queue can't find (tasks created 9-11 Nov 2026).
REM Output: lrs-capture\hidden-files-<time>.txt and .csv next to this file.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-hidden-files.ps1" -OutDir "%~dp0."
echo.
echo Finished. If you ran this from C:\Temp, copy lrs-capture\hidden-files-*.* to \\tsclient\inventory\lrs-capture and tell Claude.
pause
