# Verifies develop, commits and pushes pending changes, waits for a green GitHub CI on the
# develop tip, fast-forwards main to it, tags it and checks the CI of main. Stops at the
# first problem and says why; it never force-pushes and never creates a merge commit.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\release-main.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\release-main.ps1 -Tag phase-c-software -Message "fix(st): ..."
#   -SkipLocalTests  skip the local fmt/clippy/test/phase-check run (CI still has to pass)
# Run it in a normal PowerShell window, with no other cargo build or test running.
# Log: out\release-main.log
param(
  [string]$Tag = 'phase-c-software',
  [string]$Message = 'fix(st): a dollar before a multi-byte character no longer panics the lexer; add tools/release-main.ps1',
  [switch]$SkipLocalTests,
  [int]$CiTimeoutMinutes = 40
)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out\release-main.log'
"release-main $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
function L($m) { Write-Host $m; "$m" | Out-File -Append -Encoding utf8 $log }
function Stop-Here($m) { L ''; L "STOPPED: $m"; L 'Nothing was merged. Fix the cause and run the script again.'; exit 1 }
function Git-Ok { param([string[]]$GitArgs) $out = & git @GitArgs 2>&1 | Out-String; $code = $LASTEXITCODE; "$out".Trim() | Out-File -Append -Encoding utf8 $log; return @{ Out = "$out".Trim(); Code = $code } }

# Prepare the tools
foreach ($t in 'git', 'gh', 'cargo') {
  if (-not (Get-Command $t -ErrorAction SilentlyContinue)) { Stop-Here "$t is not on PATH" }
}
$tc = Join-Path $env:USERPROFILE 'toolchains'
foreach ($d in 'w64devkit\bin', 'avr-gcc\bin', 'avrdude\bin', 'arm-none-eabi\bin') {
  if (Test-Path "$tc\$d") { $env:PATH = "$tc\$d;$env:PATH" }
}
$env:NEURON_CC = 'gcc'; $env:NEURON_AVR_GCC = 'avr-gcc'; $env:NEURON_AVRDUDE = 'avrdude'
$env:NEURON_ARM_GCC = 'arm-none-eabi-gcc'; $env:NEURON_REQUIRE_TOOLCHAIN = '1'; $env:NEURON_REQUIRE_ARM = '1'
$env:NEURON_REQUIRE_PYTHON = '1'
$env:CARGO_BUILD_JOBS = '2'          # more ran this PC out of memory (os error 1455)
$env:CARGO_TERM_COLOR = 'never'

# 0. Nothing else may be building: two test runs collide on out\c6\driver.exe
L '== 0. other builds'
$busy = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -in 'cargo', 'rustc' }
$waited = 0
while ($busy -and $waited -lt 900) {
  if ($waited -eq 0) { L '   another cargo is running; waiting up to 15 min for it to finish...' }
  Start-Sleep 15; $waited += 15
  $busy = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -in 'cargo', 'rustc' }
}
if ($busy) { Stop-Here 'another cargo build is still running; close it (or the editor job) and retry' }
L '   none'

# 1. Branch state
L '== 1. branch state'
$b = (Git-Ok @('rev-parse', '--abbrev-ref', 'HEAD')).Out
if ($b -ne 'develop') { Stop-Here "you are on '$b'; switch to develop first (git checkout develop)" }
Git-Ok @('fetch', 'origin') | Out-Null
$behind = (Git-Ok @('rev-list', '--count', 'HEAD..origin/develop')).Out
if ($behind -ne '0') { Stop-Here "develop is $behind commit(s) behind origin/develop; run git pull --ff-only" }
$isAnc = Git-Ok @('merge-base', '--is-ancestor', 'origin/main', 'HEAD')
if ($isAnc.Code -ne 0) { Stop-Here 'main has commits that develop does not have; main cannot be fast-forwarded' }
L "   develop is up to date and main is an ancestor of it"

# 2. Local verification
if (-not $SkipLocalTests) {
  L '== 2. local verification (fmt, clippy, all tests, CLI checks): 10-25 min'
  $steps = @(
    @('fmt', 'cargo fmt --all -- --check'),
    @('clippy', 'cargo clippy --workspace --all-targets -- -D warnings'),
    @('tests', 'cargo test --workspace')
  )
  foreach ($s in $steps) {
    $p = Join-Path $root "out\release-$($s[0]).log"
    cmd /c "$($s[1]) > `"$p`" 2>&1"
    if ($LASTEXITCODE -ne 0) {
      $why = Get-Content $p | Select-String -Pattern 'FAILED|panicked|^error|os error' | Select-Object -First 8 | ForEach-Object { "      $($_.Line)" }
      $why | ForEach-Object { L $_ }
      Stop-Here "$($s[0]) failed (full output: out\release-$($s[0]).log)"
    }
    if ($s[0] -eq 'tests') {
      $pass = 0; $fail = 0
      foreach ($x in Get-Content $p) { if ($x -match 'test result: .* (\d+) passed; (\d+) failed') { $pass += [int]$Matches[1]; $fail += [int]$Matches[2] } }
      L "   tests: $pass passed, $fail failed"
    } else { L "   $($s[0]): OK" }
  }
  & powershell -NoProfile -ExecutionPolicy Bypass -File tools\phase-check.ps1 | Out-Null
  $pc = Get-Content out\phase-check.log -ErrorAction SilentlyContinue
  if (-not ($pc -match 'checks passed')) { Stop-Here 'CLI checks failed (out\phase-check.log)' }
  L '   CLI checks: OK'
} else { L '== 2. local verification skipped (-SkipLocalTests)' }

# 3. Commit and push pending changes
L '== 3. commit and push'
$dirty = (Git-Ok @('status', '--porcelain')).Out
if ($dirty) {
  $dirty -split "`n" | ForEach-Object { L "   $_" }
  Git-Ok @('add', '-A') | Out-Null
  $c = Git-Ok @('commit', '-s', '-q', '-m', $Message)
  if ($c.Code -ne 0) { Stop-Here "git commit failed: $($c.Out)" }
  L "   committed: $((Git-Ok @('log', '--oneline', '-1')).Out)"
} else { L '   nothing to commit' }
$push = Git-Ok @('push', '-q', 'origin', 'develop')
if ($push.Code -ne 0) { Stop-Here "git push failed: $($push.Out)" }
$sha = (Git-Ok @('rev-parse', 'HEAD')).Out
L "   develop pushed at $($sha.Substring(0,7))"

# 4. Wait for the GitHub CI of that exact commit
function Wait-Ci($commit, $branch, [int]$appearMinutes) {
  L "== wait for CI of ${branch} ($($commit.Substring(0,7)))"
  $deadline = (Get-Date).AddMinutes($appearMinutes)
  $id = $null
  while (-not $id -and (Get-Date) -lt $deadline) {
    $id = gh run list --commit $commit --branch $branch --limit 1 --json databaseId --jq '.[0].databaseId' 2>$null
    if (-not $id) { Start-Sleep 10 }
  }
  if (-not $id) {
    if ($branch -eq 'main') { L "   WARNING: no CI run started for main within $appearMinutes min; check https://github.com/HCHAPS404/Synapse-Studio/actions"; return }
    Stop-Here "no CI run appeared for ${branch} within $appearMinutes min"
  }
  L "   run $id; watching (this takes about 10-20 min)..."
  gh run watch $id --exit-status --interval 30 *> (Join-Path $root 'out\release-ci.log')
  $r = gh run view $id --json conclusion --jq '.conclusion' 2>$null
  if ($r -ne 'success') {
    gh run view $id --log-failed 2>$null | Select-String -Pattern 'FAILED|panicked|##\[error\]' | Select-Object -First 8 | ForEach-Object { L "      $($_.Line)" }
    if ($branch -eq 'main') { L "   WARNING: CI of main ended '$r'; main already moved, fix on develop and run again (https://github.com/HCHAPS404/Synapse-Studio/actions/runs/$id)"; return }
    Stop-Here "CI of ${branch} ended '$r' (https://github.com/HCHAPS404/Synapse-Studio/actions/runs/$id)"
  }
  L "   CI of ${branch}: success"
}
Wait-Ci $sha 'develop' $CiTimeoutMinutes

# 5. Fast-forward main, tag, push
L '== 5. promote main'
$co = Git-Ok @('checkout', '-q', 'main')
if ($co.Code -ne 0) { Stop-Here "cannot check out main: $($co.Out)" }
$ff = Git-Ok @('merge', '--ff-only', $sha)
if ($ff.Code -ne 0) { Git-Ok @('checkout', '-q', 'develop') | Out-Null; Stop-Here "main cannot be fast-forwarded: $($ff.Out)" }
$pm = Git-Ok @('push', '-q', 'origin', 'main')
if ($pm.Code -ne 0) { Git-Ok @('checkout', '-q', 'develop') | Out-Null; Stop-Here "git push main failed (is main protected against direct pushes?): $($pm.Out)" }
L "   main = $($sha.Substring(0,7)) pushed"
if ($Tag) {
  $exists = (Git-Ok @('tag', '-l', $Tag)).Out
  if ($exists) { L "   tag $Tag already exists; not moved" }
  else {
    Git-Ok @('tag', '-a', $Tag, '-m', "${Tag}: Phase C done in software, verified by CI", $sha) | Out-Null
    $pt = Git-Ok @('push', '-q', 'origin', $Tag)
    if ($pt.Code -ne 0) { L "   WARNING: tag push failed: $($pt.Out)" } else { L "   tag $Tag pushed" }
  }
}
Git-Ok @('checkout', '-q', 'develop') | Out-Null

# 6. CI of main (the merge is already done; a problem here is reported, not undone)
Wait-Ci $sha 'main' 5

L ''
L '================ DONE ================'
L "develop = main = $($sha.Substring(0,7)), tag $Tag (see the CI lines above for main)."
L 'You are back on develop. Martin can merge main into his branch.'
L 'RELEASE-MAIN DONE'
