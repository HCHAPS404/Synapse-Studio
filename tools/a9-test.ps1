# Runs the A9 (NIR) tests and writes a short summary.
# Usage: powershell -ExecutionPolicy Bypass -File tools/a9-test.ps1 [-Golden] [-Quick]
#   -Golden  regenerate conformance/golden/nir/examples.nir.json first
#   -Quick   skip the proptest (faster, about 1 minute)
# Full log: out/a9.log   Summary: out/a9-summary.txt
param([switch]$Golden, [switch]$Quick)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/a9.log'
$sum = Join-Path $root 'out/a9-summary.txt'
$env:CARGO_TERM_COLOR = 'never'

if ($Golden) {
  Write-Host 'Regenerating golden NIR...'
  $env:UPDATE_GOLDEN = '1'
  cmd /c "cargo test -p neuron-sema --test a9_nir examples_match_the_golden_nir > out\a9-golden.log 2>&1"
  Remove-Item Env:UPDATE_GOLDEN
}

$filter = ''
if ($Quick) { $filter = '-- --skip mutated' }
Write-Host 'Running A9 tests (this can take several minutes)...'
$start = Get-Date
# cmd /c avoids PowerShell 5.1 turning cargo's stderr into NativeCommandError noise.
cmd /c "cargo test -p neuron-sema --test a9_nir $filter > out\a9.log 2>&1"
$code = $LASTEXITCODE
$secs = [int]((Get-Date) - $start).TotalSeconds

$lines = Get-Content $log
$out = @("exit code: $code  ($secs s)", '')
$out += $lines | Where-Object { $_ -match '^test .* \.\.\. ' -or $_ -match '^test result:' }
$out += ''
$out += '---- first lines of each failure ----'
$in = $false; $n = 0
foreach ($l in $lines) {
  if ($l -match '^---- .* stdout ----') { $in = $true; $n = 0; $out += $l; continue }
  if ($in) {
    if ($l.Length -gt 300) { $l = $l.Substring(0, 300) + ' ...' }
    $out += $l; $n++
    if ($n -ge 25) { $in = $false; $out += '' }
  }
}
$out | Set-Content -Encoding utf8 $sum
$out | Select-Object -First 20 | ForEach-Object { Write-Host $_ }
Write-Host ''
if ($code -eq 0) { Write-Host 'RESULT: pass' -ForegroundColor Green }
else { Write-Host "RESULT: fail - see out\a9-summary.txt" -ForegroundColor Red }
