@echo off
rem Zips everything needed to rebuild AI Terminal (sources, build scripts, assets, tools, docs)
rem into backups\AI_Terminal_<date>_<time>.zip, or into the folder given as the first argument:
rem     backup.cmd                 -> backups\AI_Terminal_20261114_101500.zip
rem     backup.cmd D:\Backups      -> D:\Backups\AI_Terminal_20261114_101500.zip
rem Skips build output (target\, dist\) and earlier backups.
setlocal
cd /d "%~dp0"

set "OUTDIR=%~1"
if "%OUTDIR%"=="" set "OUTDIR=%~dp0backups"
if not exist "%OUTDIR%" mkdir "%OUTDIR%" || (echo Can't create "%OUTDIR%" & exit /b 1)

for /f %%t in ('powershell -NoProfile -Command "Get-Date -Format yyyyMMdd_HHmmss"') do set "STAMP=%%t"
set "ZIP=%OUTDIR%\AI_Terminal_%STAMP%.zip"

rem Windows' own tar.exe (bsdtar, ships with Windows 10/11): -a picks zip from the extension.
rem Called by full path so Git's GNU tar on PATH (which can't write zips) isn't used instead.
"%SystemRoot%\System32\tar.exe" -a -c -f "%ZIP%" ^
    --exclude "*/target" --exclude "*.tmp" ^
    Cargo.toml Cargo.lock .cargo build.rs src assets tools build.cmd dist.cmd build_publish_exe.cmd backup.cmd README.md HANDOFF.md .gitignore
if errorlevel 1 (
    echo Backup FAILED.
    if exist "%ZIP%" del "%ZIP%"
    exit /b 1
)

for %%f in ("%ZIP%") do echo Backup written: %%~ff  (%%~zf bytes)
endlocal
