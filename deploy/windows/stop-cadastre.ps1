# Stops the Cadastre server: the scheduled task (its restart loop) and the server process. Run elevated.
param([string]$TaskName = 'Cadastre Server')
$task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if ($task) { Stop-ScheduledTask -TaskName $TaskName }
# Ending the task ends cmd.exe, not always its child: stop the server explicitly.
Get-Process -Name 'cadastre-server' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Write-Host 'Cadastre server stopped.'
