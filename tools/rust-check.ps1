# fmt + (optional golden refresh) + clippy + tests for the Rust workspace.
# Usage: powershell -File tools/rust-check.ps1 [-Golden]
# Log: out/rust-check.log, ends with "RUST CHECK DONE".
param([switch]$Golden)
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/rust-check.log'
$env:CARGO_TERM_COLOR = 'never'
"rust-check $(Get-Date -Format o) golden=$Golden" | Out-File -Encoding utf8 $log
cargo fmt --all 2>&1 | Out-Null
if ($Golden) {
  $env:UPDATE_GOLDEN = '1'
  # Parser outlines, NIR of the examples and the NIR JSON Schema.
  $g = cargo test --workspace --quiet 2>&1 | Out-String
  "==== golden update (exit $LASTEXITCODE) ====`n$g" | Out-File -Append -Encoding utf8 $log
  Remove-Item Env:UPDATE_GOLDEN
}
$c = cargo clippy --workspace --all-targets --quiet -- -D warnings 2>&1 | Out-String
"==== clippy (exit $LASTEXITCODE) ====`n$c" | Out-File -Append -Encoding utf8 $log
$t = cargo test --workspace --quiet 2>&1 | Out-String
"==== test (exit $LASTEXITCODE) ====`n$t" | Out-File -Append -Encoding utf8 $log
"RUST CHECK DONE" | Out-File -Append -Encoding utf8 $log
