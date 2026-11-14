# Writes git status and the list of modified tracked files to out/gs.txt
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$o = @()
$o += "== status"
$o += (git status --short 2>&1 | Out-String)
$o += "== modified tracked"
$o += (git ls-files -m 2>&1 | Out-String)
$o += "== head"
$o += (git log --oneline -3 2>&1 | Out-String)
$o | Out-File -Encoding utf8 out/gs.txt
