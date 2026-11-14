# Runs the local verification suite and writes a summary to out/verify.log.
# Usage: powershell -NoProfile -File tools/verify-local.ps1
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/verify.log'
"verify started $(Get-Date -Format o)" | Out-File -Encoding utf8 $log

function Step([string]$name, [scriptblock]$cmd, [string]$cwd = $root) {
  Push-Location $cwd
  $output = & $cmd 2>&1 | Out-String
  $code = $LASTEXITCODE
  Pop-Location
  "==== $name (exit $code) ====" | Out-File -Append -Encoding utf8 $log
  $output | Out-File -Append -Encoding utf8 $log
  return $code
}

$env:NO_COLOR = '1'
$results = [ordered]@{}
$results['kiro-guard']   = Step 'kiro-guard'   { powershell -NoProfile -File tools/check-kiro-not-ignored.ps1 }
$results['cargo-fmt']    = Step 'cargo-fmt'    { cargo fmt --all -- --check }
$results['cargo-clippy'] = Step 'cargo-clippy' { cargo clippy --workspace --all-targets --quiet -- -D warnings }
$results['cargo-test']   = Step 'cargo-test'   { cargo test --workspace --quiet }
$core = Join-Path $root 'services/core'
$results['core-ruff']    = Step 'core-ruff'    { uv run --no-progress ruff check . } $core
$results['core-format']  = Step 'core-format'  { uv run --no-progress ruff format --check . } $core
$results['core-pytest']  = Step 'core-pytest'  { uv run --no-progress pytest -q -p no:cacheprovider } $core

"==== SUMMARY ====" | Out-File -Append -Encoding utf8 $log
foreach ($k in $results.Keys) { "$k : $($results[$k])" | Out-File -Append -Encoding utf8 $log }
"verify finished $(Get-Date -Format o)" | Out-File -Append -Encoding utf8 $log
