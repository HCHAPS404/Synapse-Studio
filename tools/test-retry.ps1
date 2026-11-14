# Re-run workspace tests; if App Control blocks a test exe (os error 4551),
# delete the freshly built exes and rebuild once, then retry.
$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = "out/test-retry.log"
New-Item -ItemType Directory -Force out | Out-Null
"test-retry $(Get-Date -Format o)" | Out-File $log -Encoding utf8
for ($i = 1; $i -le 3; $i++) {
  $t = cargo test --workspace --quiet 2>&1 | Out-String
  "==== attempt $i (exit $LASTEXITCODE) ====`n$t" | Out-File $log -Append -Encoding utf8
  if ($LASTEXITCODE -eq 0) { break }
  if ($t -match "4551") {
    "App Control block detected; removing test exes" | Out-File $log -Append -Encoding utf8
    Get-ChildItem target/debug/deps -Filter "*.exe" | Where-Object { $_.Name -match "^(parser|lexer|golden|lexer_rules|parser_rules)-" } | Remove-Item -Force
    Start-Sleep -Seconds 3
  } else { break }
}
"TEST RETRY DONE" | Out-File $log -Append -Encoding utf8
