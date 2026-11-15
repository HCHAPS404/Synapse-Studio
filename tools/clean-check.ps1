# Stops leftover cargo/rustc processes, deletes the build directory and runs rust-check from scratch.
# Use after a crash or a disk repair, when the incremental cache may be damaged.
# Log: out/rust-check.log (plus out/clean.log for the cleanup step).
Set-Location (Split-Path $PSScriptRoot -Parent)
New-Item -ItemType Directory -Force out | Out-Null
$o = @("clean-check $(Get-Date -Format o)")
Get-Process -Name cargo, rustc, clippy-driver, rustdoc -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep 3
if (Test-Path target) {
  try { Remove-Item -Recurse -Force target -ErrorAction Stop; $o += "target removed" }
  catch { $o += "remove failed: $($_.Exception.Message)"; $o += (cargo clean 2>&1 | Out-String) }
}
$o | Out-File -Encoding utf8 out/clean.log
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust-check.ps1
