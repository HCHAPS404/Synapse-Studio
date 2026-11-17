# Compiles (and optionally tests) one package and writes short diagnostics to out/build.log.
# Usage: powershell -File tools/build-check.ps1 -Package neuron-interp [-Test] [-Filter name] [-Clippy]
# The log ends with "BUILD DONE exit=<code>".
param([string]$Package = 'neuron-interp', [switch]$Test, [switch]$Clippy, [string]$Filter = '')
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/build.log'
$env:CARGO_TERM_COLOR = 'never'
if ($Clippy) { $cmd = "cargo clippy -p $Package --all-targets --message-format short -- -D warnings" }
elseif ($Test) { $cmd = "cargo test -p $Package --message-format short $Filter" }
else { $cmd = "cargo build -p $Package --message-format short" }
"build-check $(Get-Date -Format o): $cmd" | Out-File -Encoding utf8 $log
cmd /c "$cmd >> out\build.log 2>&1"
"BUILD DONE exit=$LASTEXITCODE" | Out-File -Append -Encoding utf8 $log
