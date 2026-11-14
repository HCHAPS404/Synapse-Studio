# Usage: powershell -File tools/grep.ps1 -Pattern "regex" -Path "glob"  (result: out/grep.txt)
param([string]$Pattern, [string]$Path)
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$r = Select-String -Path $Path -Pattern $Pattern | ForEach-Object { "$($_.Path | Split-Path -Leaf):$($_.LineNumber): $($_.Line.Trim())" }
$r | Out-File -Encoding utf8 out/grep.txt
"GREP DONE" | Out-File -Append -Encoding utf8 out/grep.txt
