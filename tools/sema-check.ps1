# Quick compile + clippy + tests for neuron-sema only. Log: out/sema-check.log ("SEMA CHECK DONE" at the end).
# Usage: powershell -File tools/sema-check.ps1 [-Test] [-Filter name] [-CleanSyntax]
param([switch]$Test, [string]$Filter = '', [switch]$CleanSyntax)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/sema-check.log'
$env:CARGO_TERM_COLOR = 'never'
"sema-check $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
if ($CleanSyntax) { cargo clean -p neuron-syntax 2>&1 | Out-Null }
cargo fmt --all 2>&1 | Out-Null
$c = cargo clippy -p neuron-sema --all-targets --quiet -- -D warnings 2>&1 | Out-String
"==== clippy (exit $LASTEXITCODE) ====`n$c" | Out-File -Append -Encoding utf8 $log
if ($Test) {
  if ($Filter) { $t = cargo test -p neuron-sema --no-fail-fast $Filter 2>&1 | Out-String }
  else { $t = cargo test -p neuron-sema --no-fail-fast 2>&1 | Out-String }
  "==== test (exit $LASTEXITCODE) ====`n$t" | Out-File -Append -Encoding utf8 $log
}
"SEMA CHECK DONE" | Out-File -Append -Encoding utf8 $log
