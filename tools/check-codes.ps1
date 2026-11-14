# Lists NE/NL codes used in spec chapters that are missing from the catalogue (chapter 10),
# with the files and lines where they occur. Output: out/codes.log
Set-Location (Split-Path $PSScriptRoot -Parent)
New-Item -ItemType Directory -Force out | Out-Null
$cat = Get-Content spec/language/10-diagnostics.md -Raw
$known = [regex]::Matches($cat, '\|\s*(N[EL]\d{4})\s*\|') | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
$out = @()
foreach ($f in Get-ChildItem spec/language/*.md, docs/errors/*.md -ErrorAction SilentlyContinue) {
  if ($f.Name -eq "10-diagnostics.md") { continue }
  $n = 0
  foreach ($line in Get-Content $f.FullName) {
    $n++
    foreach ($m in [regex]::Matches($line, 'N[EL]\d{4}')) {
      if ($known -notcontains $m.Value) { $out += "$($f.Name):${n}: $($m.Value) :: $($line.Trim())" }
    }
  }
}
if ($out.Count -eq 0) { $out = @("all codes are in the catalogue") }
$out | Out-File out/codes.log -Encoding utf8
"CODES DONE" | Out-File out/codes.log -Append -Encoding utf8
