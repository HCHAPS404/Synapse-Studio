# fmt + clippy + tests for the Rust workspace. Log: out/rust-check.log, ends with "RUST CHECK DONE".
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/rust-check.log'
$env:CARGO_TERM_COLOR = 'never'
"rust-check $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
cargo fmt --all 2>&1 | Out-Null
$c = cargo clippy --workspace --all-targets --quiet -- -D warnings 2>&1 | Out-String
"==== clippy (exit $LASTEXITCODE) ====`n$c" | Out-File -Append -Encoding utf8 $log
$t = cargo test --workspace --quiet 2>&1 | Out-String
"==== test (exit $LASTEXITCODE) ====`n$t" | Out-File -Append -Encoding utf8 $log
"RUST CHECK DONE" | Out-File -Append -Encoding utf8 $log
