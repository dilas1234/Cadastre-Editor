@echo off
REM REGISTRY USER ACTIVITY on REGISTRY (10.10.10.5). READ-ONLY: SELECT statements only, changes nothing.
REM Default: what psatdom worked on 10, 11 and 12 November 2026. Other user/dates:
REM   RUN-ME-USER-ACTIVITY.cmd someuser 2026-11-01 2026-11-08      (end date exclusive)
REM Output: lrs-capture\user-activity-<user>-<from>-<to>.txt next to this file.
set U=%1& set F=%2& set T=%3
if "%U%"=="" set U=psatdom
if "%F%"=="" set F=2026-11-10
if "%T%"=="" set T=2026-11-13
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0lrs-user-activity.ps1" -User %U% -From %F% -To %T% -OutDir "%~dp0."
echo.
echo Finished. If you ran this from C:\Temp, copy lrs-capture\user-activity-*.txt to \\tsclient\inventory\lrs-capture and tell Claude.
pause
