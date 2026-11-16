# Commit all changes with an AI-written message (one small call, no chat context), then push
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$env:GIT_CONFIG_PARAMETERS = "'core.safecrlf=false'"   # no "LF will be replaced by CRLF" noise
$env:GIT_PAGER = 'cat'

# Untracked files: mark them intent-to-add so their contents show in the diff.
$new = @(git ls-files --others --exclude-standard)
if ($new.Count) { git add -N -- $new }
$diff = (git diff HEAD | Select-Object -First 1500) -join "`n"
if (-not $diff) { Write-Host "Nothing to commit."; exit 0 }
git diff HEAD --stat

# The model (and effort) picked for this script in Options > Scripts, else Claude Haiku.
$cli = if ($env:AI_TERMINAL_CLI) { $env:AI_TERMINAL_CLI } else { 'claude' }
$model = if ($env:AI_TERMINAL_MODEL) { $env:AI_TERMINAL_MODEL } else { 'haiku' }
$effort = $env:AI_TERMINAL_EFFORT
$system = "You write git commit messages. Output only the message: a summary line under 72 characters, a blank line, then a few short bullet points. No code fences, no preamble."

$what = if ($effort) { "$cli, $model, effort $effort" } else { "$cli, $model" }
Write-Host "`nWriting the commit message ($what)..." -ForegroundColor DarkGray
if ($cli -eq 'codex') {
    $out = Join-Path $env:TEMP "ai_terminal_commit_$PID.txt"
    # Unquoted: not valid TOML, so codex reads it as a string.
    $extra = if ($effort) { @('-c', "model_reasoning_effort=$effort") } else { @() }
    "$system`n`nWrite a commit message for this diff.`n`n$diff" | codex exec -m $model @extra --skip-git-repo-check --sandbox read-only --output-last-message $out - *> $null
    $msg = if (Test-Path $out) { Get-Content $out -Encoding UTF8 } else { $null }
    Remove-Item $out -ErrorAction SilentlyContinue
} elseif ($cli -eq 'antigravity') {
    # agy -p doesn't read stdin as text: send the prompt as one stream-json message.
    $line = @{ event = 'user'; message = @{ content = "$system`n`nWrite a commit message for this diff.`n`n$diff" } } | ConvertTo-Json -Compress
    $res = $line | agy --model $model --disable-slash-commands --input-format stream-json --output-format stream-json -p= 2>$null |
        ForEach-Object { try { $_ | ConvertFrom-Json } catch {} } | Where-Object { $_.event -eq 'result' } | Select-Object -Last 1
    $msg = if ($res.result.status -eq 'SUCCESS') { $res.result.response.Trim() -split "`n" } else { $null }
} else {
    $extra = if ($effort) { @('--effort', $effort) } else { @() }
    $msg = $diff | claude -p --model $model @extra --tools '""' --strict-mcp-config --no-session-persistence --system-prompt $system "Write a commit message for this diff."
}
if ($LASTEXITCODE -ne 0 -or -not $msg) {
    Write-Host "The $cli call failed." -ForegroundColor Red
    if ($new.Count) { git reset -q -- $new }
    exit 1
}

Write-Host ""
$msg | ForEach-Object { Write-Host $_ -ForegroundColor Cyan }
Write-Host ""
Write-Host "Enter = commit and push, n = cancel"
$answer = Read-Host
if ($null -eq $answer -or $answer -eq 'n') {
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
