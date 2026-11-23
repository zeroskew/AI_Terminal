# Release build, then publish it as a GitHub Release (tag = v<date-time>)
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

$root = (git rev-parse --show-toplevel) -replace '/', '\'
if (-not $root) { Write-Host "Not inside a git repository." -ForegroundColor Red; exit 1 }
Set-Location $root

# build_publish_exe.cmd does it all: a fresh version stamp (so every run gets a new tag), a
# release build into target\release (a running dist\ai_terminal.exe doesn't block it), the zip,
# a Publish? [Y/n] question, then gh release create.
$cmd = Join-Path $root 'build_publish_exe.cmd'
if (-not (Test-Path $cmd)) { Write-Host "build_publish_exe.cmd not found in $root." -ForegroundColor Red; exit 1 }
& $cmd @args
$code = $LASTEXITCODE
if ($code -eq 0) { Write-Host "DONE" -ForegroundColor Green }
exit $code
