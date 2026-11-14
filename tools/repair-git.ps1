# Rebuilds a damaged local .git from GitHub WITHOUT touching working files.
# The damaged directory is kept as .git-broken-<timestamp> (reversible).
# Then reinstalls the Python virtual environment of services/core.
# Log: out/repair-git.log, ends with "REPAIR GIT DONE".
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/repair-git.log'
function L($m) { $m | Out-File -Append -Encoding utf8 $log }
"repair-git $(Get-Date -Format o)" | Out-File -Encoding utf8 $log

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$tmp = Join-Path $env:TEMP "synapse-clone-$stamp"
L (git clone --no-checkout https://github.com/HCHAPS404/Synapse-Studio.git $tmp 2>&1 | Out-String)
if (-not (Test-Path (Join-Path $tmp '.git'))) { L "clone failed; nothing changed"; L "REPAIR GIT DONE"; exit 1 }

Rename-Item -Path (Join-Path $root '.git') -NewName ".git-broken-$stamp"
Move-Item -Path (Join-Path $tmp '.git') -Destination (Join-Path $root '.git')
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

git config user.name "hchaps404"
git config user.email "helmut.chs@gmail.com"
git update-ref refs/heads/develop refs/remotes/origin/develop
git symbolic-ref HEAD refs/heads/develop
git branch --set-upstream-to=origin/develop develop 2>&1 | Out-Null
L (git reset --mixed 2>&1 | Out-String)
L "---- fsck ----"
L (git fsck --full 2>&1 | Out-String)
L "fsck exit: $LASTEXITCODE"
L "---- status ----"
L (git status --short 2>&1 | Out-String)
L (git log --oneline -3 2>&1 | Out-String)

Set-Location (Join-Path $root 'services/core')
L (uv sync --reinstall --no-progress 2>&1 | Out-String)
L "uv sync exit: $LASTEXITCODE"
L (uv run --no-progress ruff check . 2>&1 | Out-String)
L (uv run --no-progress pytest -q -p no:cacheprovider 2>&1 | Out-String)
L "pytest exit: $LASTEXITCODE"
L "REPAIR GIT DONE"
