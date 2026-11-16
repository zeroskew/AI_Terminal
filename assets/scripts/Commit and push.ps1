# Commit all changes with a message from Claude Haiku (one small call, no chat context), then push
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$env:GIT_CONFIG_PARAMETERS = "'core.safecrlf=false'"   # no "LF will be replaced by CRLF" noise
$env:GIT_PAGER = 'cat'

# Untracked files: mark them intent-to-add so their contents show in the diff.
$new = @(git ls-files --others --exclude-standard)
if ($new.Count) { git add -N -- $new }
$diff = (git diff HEAD | Select-Object -First 1500) -join "`n"
if (-not $diff) { Write-Host "Nothing to commit."; exit 0 }
git diff HEAD --stat

Write-Host "`nWriting the commit message..." -ForegroundColor DarkGray
$msg = $diff | claude -p --model haiku --tools '""' --strict-mcp-config --no-session-persistence `
    --system-prompt "You write git commit messages. Output only the message: a summary line under 72 characters, a blank line, then a few short bullet points. No code fences, no preamble." `
    "Write a commit message for this diff."
if ($LASTEXITCODE -ne 0 -or -not $msg) {
    Write-Host "The Claude call failed." -ForegroundColor Red
    if ($new.Count) { git reset -q -- $new }
    exit 1
}

Write-Host ""
$msg | ForEach-Object { Write-Host $_ -ForegroundColor Cyan }
Write-Host ""
if ((Read-Host "Enter = commit and push, n = cancel") -eq 'n') {
    if ($new.Count) { git reset -q -- $new }
    Write-Host "Cancelled."
    exit 0
}

git add -A
($msg -join "`n") | git commit -F -
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
git push
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "`nDONE" -ForegroundColor Green
exit 0
