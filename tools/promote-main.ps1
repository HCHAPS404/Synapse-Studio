# Fast-forwards main to develop (allowed by the branching rules: main only receives
# develop or release/*). Refuses non-fast-forward merges. Log: out/promote-main.log
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/promote-main.log'
function L($m) { $m | Out-File -Append -Encoding utf8 $log }
"promote-main $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
git fetch origin 2>&1 | Out-Null
git checkout main 2>&1 | Out-Null
L (git merge --ff-only origin/develop 2>&1 | Out-String)
L "merge exit: $LASTEXITCODE"
if ($LASTEXITCODE -eq 0) { L (git push origin main 2>&1 | Out-String); L "push exit: $LASTEXITCODE" }
git checkout develop 2>&1 | Out-Null
L (git log --oneline -3 main | Out-String)
L "done"
