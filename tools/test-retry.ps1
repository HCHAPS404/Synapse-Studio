# Runs the workspace tests on a machine where Windows Smart App Control blocks freshly built
# executables (os error 4551).
#  1. Unit and integration tests: blocked executables are deleted and the run is retried.
#  2. Doctests: rustdoc builds new executables on every run, so a different one may be blocked
#     each time. They are re-run, and every doctest must pass in at least one run.
# A test that runs and fails is never retried.
# Log: out/test-retry.log, ending with "TEST RETRY DONE" and "RESULT: pass" or "RESULT: fail".
$ErrorActionPreference = 'Continue'
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = 'out/test-retry.log'
New-Item -ItemType Directory -Force out | Out-Null
$env:CARGO_TERM_COLOR = 'never'
function L($m) { $m | Out-File $log -Append -Encoding utf8 }
"test-retry $(Get-Date -Format o)" | Out-File $log -Encoding utf8

$unit = $false
for ($i = 1; $i -le 4; $i++) {
  $t = cargo test --workspace --lib --tests --bins --quiet --no-fail-fast 2>&1 | Out-String
  $code = $LASTEXITCODE
  L "==== unit and integration, attempt $i (exit $code) ====`n$t"
  if ($code -eq 0) { $unit = $true; break }
  if ($t -match 'test result: FAILED\. \d+ passed; [1-9]') { L 'real test failure: not retried'; break }
  if ($t -notmatch '4551') { L 'failure is not a Smart App Control block: not retried'; break }
  $blocked = [regex]::Matches($t, 'deps\\([A-Za-z0-9_]+-[0-9a-f]+)\.exe') | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
  L "blocked by Smart App Control: $($blocked -join ', ')"
  foreach ($b in $blocked) { Remove-Item "target/debug/deps/$b.exe" -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 3
}

$passed = @{}; $seen = @{}; $docFail = $false
for ($i = 1; $i -le 6; $i++) {
  $t = cargo test --workspace --doc --no-fail-fast 2>&1 | Out-String
  L "==== doctests, attempt $i (exit $LASTEXITCODE) ===="
  foreach ($m in [regex]::Matches($t, '(?m)^test (.+?) \.\.\. (ok|FAILED)\s*$')) {
    $name = $m.Groups[1].Value; $seen[$name] = $true
    if ($m.Groups[2].Value -eq 'ok') { $passed[$name] = $true }
  }
  foreach ($m in [regex]::Matches($t, '(?ms)^---- (.+?) stdout ----\r?\n(.*?)(?=^---- |\z)')) {
    if ($m.Groups[2].Value -notmatch "Couldn't run the test") { $docFail = $true; L "real doctest failure: $($m.Groups[1].Value)" }
  }
  $pending = @($seen.Keys | Where-Object { -not $passed.ContainsKey($_) })
  L "doctests seen=$($seen.Count) passed=$($passed.Count) pending=$($pending -join '; ')"
  if ($docFail -or ($seen.Count -gt 0 -and $pending.Count -eq 0)) { break }
}
$doc = (-not $docFail) -and $seen.Count -gt 0 -and (@($seen.Keys | Where-Object { -not $passed.ContainsKey($_) }).Count -eq 0)

L 'TEST RETRY DONE'
L "unit_and_integration=$unit doctests=$doc"
if ($unit -and $doc) { L 'RESULT: pass' } else { L 'RESULT: fail' }
