# Read-only health check of the whole repository. It never commits, pushes or edits tracked files.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\diagnose.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\diagnose.ps1 -Quick   # skips the slow checks (steps 4 to 6)
# Steps: 1 git state and sync with origin, 2 tags and branches, 3 every tracked file against its blob,
# 4 cargo fmt + clippy, 5 cargo test --workspace, 6 tools\phase-check.ps1 (CLI checks of CI, no commit).
# Output: console and out\diagnose.log (summary at the end). Takes 10-25 minutes without -Quick.
param([switch]$Quick)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out\diagnose.log'
"diagnose $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
$tc = Join-Path $env:USERPROFILE 'toolchains'
foreach ($d in 'w64devkit\bin', 'avr-gcc\bin', 'avrdude\bin', 'arm-none-eabi\bin') {
  if (Test-Path "$tc\$d") { $env:PATH = "$tc\$d;$env:PATH" }
}
$env:NEURON_CC = 'gcc'; $env:NEURON_AVR_GCC = 'avr-gcc'; $env:NEURON_AVRDUDE = 'avrdude'
$env:NEURON_ARM_GCC = 'arm-none-eabi-gcc'; $env:NEURON_REQUIRE_TOOLCHAIN = '1'; $env:NEURON_REQUIRE_ARM = '1'
$env:CARGO_TERM_COLOR = 'never'
$env:CARGO_TERM_PROGRESS_WHEN = 'never'
$results = [ordered]@{}
function L($m) { Write-Host $m; "$m" | Out-File -Append -Encoding utf8 $log }
function Run($name, $cmd, $logName) {
  L "== $name"
  $p = Join-Path $root "out\$logName"
  cmd /c "($cmd) > `"$p`" 2>&1"
  $code = $LASTEXITCODE
  $script:results[$name] = if ($code -eq 0) { 'OK' } else { "FAIL (exit $code, see out\$logName)" }
  L "   -> $($script:results[$name])"
  return $p
}

# 1. Git state
L '== 1. git state'
git fetch origin 2>&1 | Out-Null
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
$ahead = (git rev-list --count "origin/$branch..HEAD" 2>$null)
$behind = (git rev-list --count "HEAD..origin/$branch" 2>$null)
$dirty = @(git status --porcelain)
L "   branch=$branch head=$(git rev-parse --short HEAD) ahead=$ahead behind=$behind uncommitted=$($dirty.Count)"
$dirty | ForEach-Object { L "   $_" }
$results['1 git sync'] = if ($ahead -eq '0' -and $behind -eq '0') { 'OK' } else { "ATTENTION ahead=$ahead behind=$behind" }
$results['1 uncommitted files'] = if ($dirty.Count -eq 0) { 'OK' } else { "ATTENTION $($dirty.Count) file(s), listed above" }

# 2. Branches and tags
L '== 2. branches and tags'
foreach ($ref in 'develop', 'main') {
  $l = (git rev-parse --short $ref 2>$null); $r = (git rev-parse --short "origin/$ref" 2>$null)
  L "   $ref local=$l origin=$r"
  $results["2 $ref = origin/$ref"] = if ($l -and $l -eq $r) { 'OK' } else { 'ATTENTION differs or missing' }
}
foreach ($t in 'spec-v1.0-rc1', 'phase-b-software') {
  $ok = [bool](git tag -l $t)
  $results["2 tag $t"] = if ($ok) { 'OK' } else { 'MISSING' }
}
L "   tags: $((git tag) -join ', ')"

# 3. Tracked files against git
L '== 3. every tracked file against its committed blob'
& powershell -NoProfile -ExecutionPolicy Bypass -File tools\verify-worktree.ps1 | Out-Null
$v = Get-Content out\verify-worktree.log
$v | Select-Object -Skip 1 | ForEach-Object { L "   $_" }
$bad = ($v | Where-Object { $_ -match '^(DIFFERENT|MISSING)' })
$results['3 files match git'] = if (-not $bad) { 'OK' } else { "ATTENTION $(@($bad).Count) file(s), listed above" }

if (-not $Quick) {
  # 4. fmt + clippy
  Run '4a cargo fmt --check' 'cargo fmt --all -- --check' 'diagnose-fmt.log' | Out-Null
  Run '4b cargo clippy' 'cargo clippy --workspace --all-targets -- -D warnings' 'diagnose-clippy.log' | Out-Null

  # 5. Unit and integration tests
  $tl = Run '5 cargo test --workspace' 'cargo test --workspace' 'diagnose-tests.log'
  $lines = Get-Content $tl -ErrorAction SilentlyContinue
  $passed = 0; $failed = 0
  foreach ($x in $lines) { if ($x -match 'test result: .* (\d+) passed; (\d+) failed') { $passed += [int]$Matches[1]; $failed += [int]$Matches[2] } }
  L "   tests passed=$passed failed=$failed"
  $results['5 test count'] = "passed=$passed failed=$failed"

  # 6. CLI checks of CI (no commit)
  & powershell -NoProfile -ExecutionPolicy Bypass -File tools\phase-check.ps1 | Out-Null
  $pc = Get-Content out\phase-check.log
  $pc | Select-Object -Skip 1 | ForEach-Object { L "   $_" }
  $results['6 CLI checks (examples, corpus, lib)'] = if ($pc -match 'checks passed') { 'OK' } else { 'FAIL, see out\phase-check.log' }
}

L ''
L '================ SUMMARY ================'
foreach ($k in $results.Keys) { L ("{0,-40} {1}" -f $k, $results[$k]) }
$all = ($results.Values | Where-Object { $_ -notmatch '^(OK|passed=\d+ failed=0)$' })
L ''
L $(if (-not $all) { 'ALL GOOD' } else { "NOT ALL GOOD: $(@($all).Count) item(s) need a look" })
L 'DIAGNOSE DONE'
