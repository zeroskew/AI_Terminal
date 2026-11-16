@echo off
rem Release build -> ai_terminal.zip -> GitHub Release (tag = v<build version>).
rem     build_publish_exe.cmd        asks before publishing
rem     build_publish_exe.cmd -y     publishes without asking
rem The tag is the exe's own version stamp (UTC build time, see build.rs), so the app's
rem self-update sees every published build as newer. Needs the GitHub CLI (gh), logged in.
setlocal
cd /d "%~dp0"

where gh >nul 2>nul || (echo GitHub CLI ^(gh^) is not installed. & exit /b 1)
gh auth status >nul 2>nul || (echo gh is not logged in - run: gh auth login & exit /b 1)

rem Builds straight into target\release, not via build.cmd/dist.cmd: dist\ isn't touched, so
rem a running dist\ai_terminal.exe doesn't block the build.
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>nul
cargo build --release || (echo Build FAILED. & exit /b 1)

set "EXE=%~dp0target\release\ai_terminal.exe"
set "VERSION="
for /f "delims=" %%v in ('"%EXE%" --version') do set "VERSION=%%v"
echo %VERSION%| findstr /r "^[0-9][0-9][0-9][0-9]\.[0-9][0-9]\.[0-9][0-9]-[0-9][0-9][0-9][0-9]$" >nul
if errorlevel 1 (echo target\release\ai_terminal.exe doesn't report a version ^(got "%VERSION%"^). & exit /b 1)
set "TAG=v%VERSION%"

gh release view %TAG% >nul 2>nul && (echo Release %TAG% already exists - build again for a new version. & exit /b 1)

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
if errorlevel 1 (echo Zip FAILED. & rmdir /s /q "%STAGE%" & exit /b 1)

for %%f in ("%ZIP%") do echo Release %TAG%  (ai_terminal.zip, %%~zf bytes)
if /i not "%~1"=="-y" (
    set "ANSWER="
    set /p "ANSWER=Publish? [Y/n] "
    call :cancelled && (rmdir /s /q "%STAGE%" & echo Cancelled. & exit /b 0)
)

gh release create %TAG% "%ZIP%" --title %TAG% --generate-notes
set "CODE=%ERRORLEVEL%"
rmdir /s /q "%STAGE%"
if not "%CODE%"=="0" (echo Publish FAILED. & exit /b %CODE%)

echo.
echo Published %TAG%
echo Download: https://github.com/zeroskew/AI_Terminal/releases/latest/download/ai_terminal.zip
endlocal
exit /b 0

:cancelled
rem Succeeds (errorlevel 0) when the answer was n/no.
if /i "%ANSWER%"=="n" exit /b 0
if /i "%ANSWER%"=="no" exit /b 0
exit /b 1
