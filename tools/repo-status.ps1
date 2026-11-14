# Writes repository status to out/repo-status.log (used by agents whose terminal
# output is unreliable).
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/repo-status.log'
"uv.lock present: $(Test-Path services/core/uv.lock)" | Out-File -Encoding utf8 $log
"Cargo.lock present: $(Test-Path Cargo.lock)" | Out-File -Append -Encoding utf8 $log
"---- git status ----" | Out-File -Append -Encoding utf8 $log
git status --short --untracked-files=all 2>&1 | Out-File -Append -Encoding utf8 $log
