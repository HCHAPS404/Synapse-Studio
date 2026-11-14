# Checks git health, cleans the Rust build and runs the full local verification.
# Log: out/repair.log (then out/verify.log). Marker line "REPAIR DONE" at the end.
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/repair.log'
"repair $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
$fsck = git fsck --full 2>&1 | Out-String
"---- git fsck (exit $LASTEXITCODE) ----`n$fsck" | Out-File -Append -Encoding utf8 $log
cargo clean 2>&1 | Out-Null
"cargo clean exit $LASTEXITCODE" | Out-File -Append -Encoding utf8 $log
powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'tools/verify-local.ps1')
Get-Content (Join-Path $root 'out/verify.log') | Select-String -Pattern ' : ' | Out-File -Append -Encoding utf8 $log
"REPAIR DONE" | Out-File -Append -Encoding utf8 $log
