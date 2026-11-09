@echo off
REM CHARTING CAPTURE on PRODUCTIONMC - double-click (Run as administrator) inside the Remote Desktop
REM session. READ-ONLY on this server: it copies the Cadastral_Maps documents, the old sheet scans,
REM their geodatabases and the report form, up to 60 GB.
REM Output: \\tsclient\inventory\aumentum-capture\charting\ (back on the Linux machine).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aumentum-charting.ps1" -OutDir "%~dp0."
echo.
echo Finished. Keep this window open if it shows anything in red, and tell Claude.
pause
