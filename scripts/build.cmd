@echo off
rem Ejecuta scripts\build.ps1 sin tocar la politica de ejecucion de PowerShell.
rem Acepta las mismas opciones: -Updater  -SkipInstall  -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" %*
exit /b %ERRORLEVEL%
