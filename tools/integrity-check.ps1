# Reports files modified vs HEAD and whether text files contain NUL/binary bytes.
# Log: out/integrity.log
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out/integrity.log'
"integrity $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
"---- git status ----" | Out-File -Append -Encoding utf8 $log
git status --short --untracked-files=all 2>&1 | Out-File -Append -Encoding utf8 $log
"---- binary check (tracked + untracked text files) ----" | Out-File -Append -Encoding utf8 $log
$files = git ls-files --cached --others --exclude-standard
foreach ($f in $files) {
  if ($f -match '\.(md|rs|py|toml|yml|yaml|json|ps1|neuron|ebnf|svg|txt|lock|example)$' -or $f -match '(LICENSE|NOTICE|CODEOWNERS|\.gitignore|\.gitattributes|\.editorconfig)$') {
    $bytes = [System.IO.File]::ReadAllBytes((Join-Path $root $f))
    $bad = 0
    foreach ($b in $bytes) { if ($b -eq 0 -or ($b -lt 9) -or ($b -gt 13 -and $b -lt 32)) { $bad++ } }
    if ($bad -gt 0) { "CORRUPT? $f  ($bad control bytes of $($bytes.Length))" | Out-File -Append -Encoding utf8 $log }
  }
}
"---- HEAD version of requirements.md (first 5 lines) ----" | Out-File -Append -Encoding utf8 $log
git show HEAD:.kiro/specs/neuron-language/requirements.md 2>&1 | Select-Object -First 5 | Out-File -Append -Encoding utf8 $log
"done" | Out-File -Append -Encoding utf8 $log
