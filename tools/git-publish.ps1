# Step 1 (check): configure repo-local identity, add remote, inspect remote. Writes out/git-publish.log
param([switch]$Push)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/git-publish.log'
function L($m) { $m | Out-File -Append -Encoding utf8 $log }
"git-publish $(Get-Date -Format o) push=$Push" | Out-File -Encoding utf8 $log

git config user.name "hchaps404"
git config user.email "helmut.chs@gmail.com"
$remotes = git remote
if ($remotes -notcontains 'origin') { git remote add origin https://github.com/HCHAPS404/Synapse-Studio.git }
L "remote: $(git remote get-url origin)"
L "---- ls-remote ----"
L (git ls-remote origin 2>&1 | Out-String)
L "ls-remote exit: $LASTEXITCODE"

if ($Push) {
  git add -A 2>&1 | Out-Null
  L "---- staged ----"
  L (git status --short | Out-String)
  L (git commit -s -m "chore: project foundation (F0): repo, Kiro steering/hooks/spec, CI, Rust workspace, Core API" 2>&1 | Out-String)
  L "commit exit: $LASTEXITCODE"
  L (git push -u origin main 2>&1 | Out-String)
  L "push main exit: $LASTEXITCODE"
  git branch develop 2>&1 | Out-Null
  L (git push -u origin develop 2>&1 | Out-String)
  L "push develop exit: $LASTEXITCODE"
  L (git log --oneline -3 | Out-String)
}
L "done"
