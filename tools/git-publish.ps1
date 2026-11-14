# Commits all changes (DCO sign-off) and pushes the current branch.
# Writes the result to out/git-publish.log (the integrated terminal truncates output).
# Usage: powershell -NoProfile -File tools/git-publish.ps1 -Message "docs: ..." [-Branch main]
param(
  [Parameter(Mandatory = $true)][string]$Message,
  [string]$Branch = ''
)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/git-publish.log'
function L($m) { $m | Out-File -Append -Encoding utf8 $log }
"git-publish $(Get-Date -Format o)" | Out-File -Encoding utf8 $log

if ($Branch) { git checkout $Branch 2>&1 | Out-Null }
$current = git rev-parse --abbrev-ref HEAD
L "branch: $current"
git add -A 2>&1 | Out-Null
L (git status --short | Out-String)
L (git commit -s -m $Message 2>&1 | Out-String)
L "commit exit: $LASTEXITCODE"
L (git push origin $current 2>&1 | Out-String)
L "push exit: $LASTEXITCODE"
L (git log --oneline -3 | Out-String)
L "done"
