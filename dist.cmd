@echo off
rem Builds the release binary; build.cmd copies it (and README.md) into dist\.
cd /d "%~dp0"
rem Close every running AI Terminal (and its child processes) so the exe is not locked.
tasklist /fi "imagename eq ai_terminal.exe" 2>nul | find /i "ai_terminal.exe" >nul && (
    echo Closing running AI Terminal processes...
    taskkill /f /t /im ai_terminal.exe >nul 2>nul
    rem Give Windows a moment to release the file lock.
    ping -n 2 127.0.0.1 >nul
)
call "%~dp0build.cmd" build --release || exit /b 1
echo dist\ ready:
dir /b dist
