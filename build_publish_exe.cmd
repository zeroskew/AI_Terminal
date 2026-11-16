@echo off
rem Release build -> ai_terminal.zip -> GitHub Release (tag = v<build version>).
rem     build_publish_exe.cmd        asks before publishing
rem     build_publish_exe.cmd -y     publishes without asking
rem The tag is the exe's own version stamp (UTC build time, see build.rs), so the app's
rem self-update sees every published build as newer. Needs the GitHub CLI (gh), logged in.
setlocal
cd /d "%~dp0"

rem Double-clicked from Explorer: pause before exiting so the window doesn't vanish with the
rem result. Explorer runs cmd.exe /c ""<script>" " (as does PowerShell, hence the parent check:
rem powershell -> for /f's cmd -> this cmd -> its parent).
set "HOLD="
set "CL=%cmdcmdline:"=#%"
if "%CL:/c ##=%"=="%CL%" goto :checked
for /f %%p in ('powershell -NoLogo -NoProfile -Command "$p=$PID; 1..3 | %% { $p=(Get-CimInstance Win32_Process -Filter ('ProcessId='+$p)).ParentProcessId }; (Get-Process -Id $p).ProcessName"') do if /i "%%p"=="explorer" set "HOLD=1"
:checked

where gh >nul 2>nul || (echo GitHub CLI ^(gh^) is not installed. & set "RC=1" & goto :done)
gh auth status >nul 2>nul || (echo gh is not logged in - run: gh auth login & set "RC=1" & goto :done)

rem A fresh version stamp (UTC now) on every run: build.rs only re-stamps when the sources
rem change, so without this an unchanged tree would rebuild nothing and keep an already
rem published version. build.rs reruns when AI_TERMINAL_VERSION changes.
set "AI_TERMINAL_VERSION="
for /f %%t in ('powershell -NoLogo -NoProfile -Command "[DateTime]::UtcNow.ToString('yyyy.MM.dd-HHmm')"') do set "AI_TERMINAL_VERSION=%%t"
if not defined AI_TERMINAL_VERSION (echo Can't read the current time. & set "RC=1" & goto :done)

rem Builds straight into target\release, not via build.cmd/dist.cmd: dist\ isn't touched, so
rem a running dist\ai_terminal.exe doesn't block the build.
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>nul
cargo build --release || (echo Build FAILED. & set "RC=1" & goto :done)

set "EXE=%~dp0target\release\ai_terminal.exe"
set "VERSION="
for /f "delims=" %%v in ('"%EXE%" --version') do set "VERSION=%%v"
echo %VERSION%| findstr /r "^[0-9][0-9][0-9][0-9]\.[0-9][0-9]\.[0-9][0-9]-[0-9][0-9][0-9][0-9]$" >nul
if errorlevel 1 (echo target\release\ai_terminal.exe doesn't report a version ^(got "%VERSION%"^). & set "RC=1" & goto :done)
set "TAG=v%VERSION%"

gh release view %TAG% >nul 2>nul && (echo Release %TAG% already exists - wait a minute and run again. & set "RC=1" & goto :done)

rem The release tag is created on the remote's default branch, so local changes aren't in it.
git status --porcelain | findstr . >nul && echo Warning: uncommitted changes in the working tree.
git log @{u}.. --oneline 2>nul | findstr . >nul && echo Warning: unpushed commits - the tag will point at the remote HEAD.

rem The asset must be named ai_terminal.zip (src\update.rs and the README link look for it).
set "STAGE=%TEMP%\ai_terminal_release"
if exist "%STAGE%" rmdir /s /q "%STAGE%"
mkdir "%STAGE%" || (echo Can't create "%STAGE%" & exit /b 1)
set "ZIP=%STAGE%\ai_terminal.zip"
rem Windows' own tar.exe (bsdtar) by full path: Git's GNU tar can't write zips.
"%SystemRoot%\System32\tar.exe" -a -c -f "%ZIP%" -C "%~dp0target\release" ai_terminal.exe
if errorlevel 1 (echo Zip FAILED. & rmdir /s /q "%STAGE%" & set "RC=1" & goto :done)

for %%f in ("%ZIP%") do echo Release %TAG%  (ai_terminal.zip, %%~zf bytes)
if /i not "%~1"=="-y" (
    set "ANSWER="
    set /p "ANSWER=Publish? [Y/n] "
    call :cancelled && (rmdir /s /q "%STAGE%" & echo Cancelled. & set "RC=0" & goto :done)
)

gh release create %TAG% "%ZIP%" --title %TAG% --generate-notes
set "RC=%ERRORLEVEL%"
rmdir /s /q "%STAGE%"
if not "%RC%"=="0" (echo Publish FAILED. & goto :done)

echo.
echo Published %TAG%
echo Download: https://github.com/zeroskew/AI_Terminal/releases/latest/download/ai_terminal.zip
set "RC=0"

:done
if defined HOLD pause
endlocal & exit /b %RC%

:cancelled
rem Succeeds (errorlevel 0) when the answer was n/no.
if /i "%ANSWER%"=="n" exit /b 0
if /i "%ANSWER%"=="no" exit /b 0
exit /b 1
