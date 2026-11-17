<#
.SYNOPSIS
  Checks, formats (optionally) and tests every package of examples/corpus.
.DESCRIPTION
  Writes out/corpus.log, keeping only errors, results and failed tests (warnings are
  dropped). With -Fix it rewrites the files in the canonical layout first.
  Usage: powershell -ExecutionPolicy Bypass -File tools\corpus-check.ps1 [-Fix]
#>
param([switch]$Fix)

$ErrorActionPreference = 'Continue'
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force out | Out-Null
$log = 'out/corpus.log'
"corpus-check $(Get-Date -Format o) fix=$Fix" | Set-Content $log

# Keep the lines that matter: errors with their location, per-file results, test results.
function Keep([string]$text) {
  $keep = $false
  foreach ($line in ($text -split "`r?`n")) {
    if ($line -match '^\s*(error|warning)') { $keep = $line -match '^\s*error' }
    if ($keep -or $line -match ': ok$|error\(s\)$|^(PASS|FAIL)|would reformat|changed,|test\(s\)') {
      if ($line -notmatch 'CategoryInfo|FullyQualifiedErrorId|^En E:|^\+') { $line }
    }
  }
}

cargo build -q -p neuron-cli 2>&1 | Out-Null
$exe = 'target/debug/neuron.exe'
foreach ($dir in Get-ChildItem examples/corpus -Directory) {
  $d = "examples/corpus/$($dir.Name)"
  "==== $d" | Add-Content $log
  if ($Fix) { Keep (& $exe fmt $d 2>&1 | Out-String) | Add-Content $log }
  Keep (& $exe check $d 2>&1 | Out-String) | Add-Content $log
  "check exit=$LASTEXITCODE" | Add-Content $log
  Keep (& $exe fmt --check $d 2>&1 | Out-String) | Add-Content $log
  "fmt exit=$LASTEXITCODE" | Add-Content $log
  Keep (& $exe test $d 2>&1 | Out-String) | Add-Content $log
  "test exit=$LASTEXITCODE" | Add-Content $log
}
'CORPUS DONE' | Add-Content $log
