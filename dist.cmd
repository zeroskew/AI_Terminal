@echo off
rem Builds the release binary; build.cmd copies it (and README.md) into dist\.
cd /d "%~dp0"
rem Close every running AI Terminal (and its child processes) so the exe is not locked.
tasklist /fi "imagename eq ai_terminal.exe" 2>nul | find /i "ai_terminal.exe" >nul && (
    echo Closing running AI Terminal processes...
    taskkill /f /t /im ai_terminal.exe >nul 2>nul
)
call :wait_exit ai_terminal.exe || (
    echo ai_terminal.exe is still running - close it and try again.
    exit /b 1
)
call "%~dp0build.cmd" build --release || exit /b 1
echo dist\ ready:
dir /b dist
exit /b 0

:wait_exit
rem Waits until no %1 process is left, so its exe can be overwritten.
rem Gives up after 20 checks: ~20 s, or ~2 s when timeout has no console and returns at once.
for /l %%i in (1,1,20) do (
    tasklist /fi "imagename eq %~1" /nh 2>nul | find /i "%~1" >nul || exit /b 0
    timeout /t 1 /nobreak >nul 2>nul
)
exit /b 1
