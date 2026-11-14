# Dumps every NE/NL code usage in chapters 01-09 and 11-14 so meanings can be compared
# with the catalogue. Output: out/codes-usage.log
Set-Location (Split-Path $PSScriptRoot -Parent)
New-Item -ItemType Directory -Force out | Out-Null
$out = @()
foreach ($f in Get-ChildItem spec/language/*.md) {
  if ($f.Name -match "^(10-|README)") { continue }
  $n = 0
  foreach ($line in Get-Content $f.FullName) {
    $n++
    if ($line -match 'N[EL]\d{4}') { $out += "$($f.Name):${n}: $($line.Trim())" }
  }
}
$out | Out-File out/codes-usage.log -Encoding utf8
"USAGE DONE" | Out-File out/codes-usage.log -Append -Encoding utf8
