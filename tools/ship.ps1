# Checks everything, then commits and pushes the current branch ONLY if all checks pass.
#   powershell -File tools/ship.ps1 -Message "feat(x): ..." [-Golden] [-AllowDelete]
# Guards, in order (each one stops the script before anything is committed):
#   1. no tracked file is deleted (unless -AllowDelete): a damaged disk once made a commit that
#      deleted half the repository;
#   2. no text file contains NUL bytes (a sign of a damaged disk);
#   3. cargo fmt, clippy -D warnings and every test pass (rust-check.ps1);
# Log: out/ship.log. The last line is "SHIP DONE: pushed" or "SHIP DONE: stopped (<why>)".
param(
  [Parameter(Mandatory = $true)][string]$Message,
  [switch]$Golden,
  [switch]$AllowDelete
)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/ship.log'
"ship $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
function L($m) { $m | Out-File -Append -Encoding utf8 $log }
function Stop-Ship($why) { L "SHIP DONE: stopped ($why)"; exit 1 }

$branch = (git rev-parse --abbrev-ref HEAD).Trim()
L "branch: $branch"
$status = git status --porcelain
L ($status | Out-String)

# 1. deletions
$deleted = $status | Where-Object { $_ -match '^( D|D |AD|DD)' }
if ($deleted -and -not $AllowDelete) { L ($deleted | Out-String); Stop-Ship 'tracked files are deleted; use -AllowDelete if that is intended' }

# 2. NUL bytes in changed text files
$bad = @()
foreach ($line in $status) {
  if ($line.Length -lt 4) { continue }
  $p = $line.Substring(3).Trim('"')
  if ($p -match '\.(rs|md|toml|json|yml|yaml|ps1|neuron|ebnf|txt)$' -and (Test-Path -LiteralPath $p -PathType Leaf)) {
    $b = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $p))
    if ($b -contains 0) { $bad += $p }
  }
}
if ($bad.Count -gt 0) { L ($bad -join "`n"); Stop-Ship 'files with NUL bytes (damaged disk?)' }

# 3. checks
Get-Process cargo, rustc, clippy-driver -ErrorAction SilentlyContinue | Stop-Process -Force
if ($Golden) { powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust-check.ps1 -Golden }
else { powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust-check.ps1 }
$r = Get-Content out/rust-check.log
L (($r | Where-Object { $_ -cmatch '^==== |test result: FAILED|panicked|^error' }) | Out-String)
$clippyOk = [bool]($r -cmatch '^==== clippy \(exit 0\) ====')
$testOk = [bool]($r -cmatch '^==== test \(exit 0\) ====')
$failed = [bool]($r -cmatch 'test result: FAILED|panicked at')
if (-not $clippyOk -or -not $testOk -or $failed) { Stop-Ship 'fmt, clippy or tests failed (see out/rust-check.log)' }

# 4. commit and push
git add -A 2>&1 | Out-Null
L (git commit -s -m $Message 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) { Stop-Ship 'nothing to commit or commit failed' }
L (git push origin $branch 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) { Stop-Ship 'push failed' }
L (git log --oneline -3 | Out-String)
L 'SHIP DONE: pushed'
