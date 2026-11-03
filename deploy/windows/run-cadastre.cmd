@echo off
rem Runs cadastre-server.exe with settings from cadastre.env, restarting it if it exits.
rem Started at boot by the "Cadastre Server" scheduled task (install-cadastre.ps1).
setlocal DisableDelayedExpansion
cd /d "%~dp0"
if not exist logs mkdir logs

for /f "usebackq eol=# tokens=1,* delims==" %%a in ("cadastre.env") do set "%%a=%%b"

:loop
echo [%date% %time%] starting cadastre-server >> logs\cadastre-server.log
cadastre-server.exe >> logs\cadastre-server.log 2>&1
echo [%date% %time%] cadastre-server exited with %errorlevel%, restarting in 10 s >> logs\cadastre-server.log
rem ping as a sleep: "timeout" fails when there is no console (scheduled task).
ping -n 11 127.0.0.1 > nul
goto loop
