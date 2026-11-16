# Publish dist\ai_terminal.exe as a GitHub Release (tag = v<date-time>)
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

$root = (git rev-parse --show-toplevel) -replace '/', '\'
if (-not $root) { Write-Host "Not inside a git repository." -ForegroundColor Red; exit 1 }
Set-Location $root

$exe = Join-Path $root 'dist\ai_terminal.exe'
if (-not (Test-Path $exe)) { Write-Host "dist\ai_terminal.exe not found - run a release build first." -ForegroundColor Red; exit 1 }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Write-Host "GitHub CLI (gh) is not installed." -ForegroundColor Red; exit 1 }

$tag = 'v' + (Get-Date -Format 'yyyy.MM.dd-HHmm')   # every publish is a new release, no manual version bump

# The tag must point at pushed code
if (git status --porcelain) { Write-Host "Warning: uncommitted changes in the working tree." -ForegroundColor Yellow }
if (git log '@{u}..' --oneline 2>$null) { Write-Host "Warning: unpushed commits - the release tag will point at the remote HEAD." -ForegroundColor Yellow }

$zip = Join-Path $env:TEMP 'ai_terminal.zip'
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $exe -DestinationPath $zip -CompressionLevel Optimal
$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "Release $tag  (ai_terminal.zip, $size MB)" -ForegroundColor Cyan
Write-Host "Enter = publish, n = cancel"
$answer = Read-Host
if ($null -eq $answer -or $answer -eq 'n') { Remove-Item $zip -ErrorAction SilentlyContinue; Write-Host "Cancelled."; exit 0 }

gh release create $tag $zip --title $tag --generate-notes
$code = $LASTEXITCODE
Remove-Item $zip -ErrorAction SilentlyContinue
if ($code -ne 0) { exit $code }

Write-Host "`nDownload: https://github.com/zeroskew/AI_Terminal/releases/latest/download/ai_terminal.zip" -ForegroundColor Green
Write-Host "DONE" -ForegroundColor Green
exit 0
