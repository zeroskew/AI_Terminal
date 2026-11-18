# Commit and push (AI-written message), then release build and publish the exe as a GitHub Release
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$env:GIT_PAGER = 'cat'
# The AI model for the commit message comes from Options > Scripts ($env:AI_TERMINAL_MODEL),
# passed on to "Commit and push.ps1", which runs in this same process.

$root = (git rev-parse --show-toplevel) -replace '/', '\'
if (-not $root) { Write-Host "Not inside a git repository." -ForegroundColor Red; exit 1 }
Set-Location $root

$commit = Join-Path $PSScriptRoot 'Commit and push.ps1'
if (-not (Test-Path $commit)) { Write-Host "Commit and push.ps1 not found next to this script." -ForegroundColor Red; exit 1 }
$cmd = Join-Path $root 'build_publish_exe.cmd'
if (-not (Test-Path $cmd)) { Write-Host "build_publish_exe.cmd not found in $root." -ForegroundColor Red; exit 1 }

# 1. Commit and push. It exits 0 for "nothing to commit" and for "cancelled" alike, so the
#    working tree tells them apart: anything left uncommitted means stop here.
Write-Host "== Commit and push" -ForegroundColor Yellow
& $commit
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Set-Location $root
if (git status --porcelain) { Write-Host "Uncommitted changes left - not publishing." -ForegroundColor Red; exit 1 }

# Nothing new to commit but older commits not pushed yet: push them, since the release tag is
# made on the remote's HEAD.
if (git log '@{u}..' --oneline 2>$null) {
    git push
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# 2. Build and publish: fresh version stamp, release build, zip, Publish? [Y/n], gh release create.
Write-Host "`n== Build and publish" -ForegroundColor Yellow
& $cmd @args
$code = $LASTEXITCODE
if ($code -eq 0) { Write-Host "DONE" -ForegroundColor Green }
exit $code
