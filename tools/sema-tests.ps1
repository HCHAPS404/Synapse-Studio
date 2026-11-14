# Runs each neuron-sema test binary separately with a timeout and streams its output.
# Usage: powershell -File tools/sema-tests.ps1 [-Only a5_types] [-Seconds 240]
# Logs: out/test-<name>.log and the summary out/sema-tests.log ("SEMA TESTS DONE" at the end).
param([string]$Only = '', [int]$Seconds = 240)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$env:CARGO_TERM_COLOR = 'never'
$summary = Join-Path $root 'out/sema-tests.log'
"sema-tests $(Get-Date -Format o)" | Out-File -Encoding utf8 $summary
cargo test -p neuron-sema --no-run 2>&1 | Out-File -Encoding utf8 (Join-Path $root 'out/test-build.log')
$targets = @('--lib', '--test a4_names', '--test a5_types', '--test a6_rules', '--test a7_time', '--test a8_safety')
foreach ($t in $targets) {
  $name = ($t -replace '--test ', '' -replace '--', '')
  if ($Only -and $name -ne $Only) { continue }
  $log = Join-Path $root "out/test-$name.log"
  $args = "test -p neuron-sema $t --no-fail-fast"
  $job = Start-Job -ScriptBlock { param($dir, $a, $l) Set-Location $dir; $env:CARGO_TERM_COLOR = 'never'; Invoke-Expression "cargo $a 2>&1" | Out-File -Encoding utf8 $l } -ArgumentList $root, $args, $log
  if (Wait-Job $job -Timeout $Seconds) { "$name finished" | Out-File -Append -Encoding utf8 $summary }
  else { Stop-Job $job; Get-Process -Name "a*_*", "neuron_sema*" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue; "$name TIMEOUT after $Seconds s" | Out-File -Append -Encoding utf8 $summary }
  Remove-Job $job -Force
}
"SEMA TESTS DONE" | Out-File -Append -Encoding utf8 $summary
