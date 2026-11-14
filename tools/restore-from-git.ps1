# Restores the given paths from the index/HEAD (never touches other files).
# -Force deletes the working copy first, which is needed when the file is silently corrupted
# (git skips files whose timestamp and size did not change).
# Usage: powershell -File tools/restore-from-git.ps1 -Paths examples/thermostat.neuron,services/core [-Force]
# Log: out/restore.log
param([string[]]$Paths, [switch]$Force)
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$o = @("restore $(Get-Date -Format o) force=$Force")
foreach ($p in ($Paths | ForEach-Object { $_ -split ',' })) {
  $o += "== $p"
  if ($Force -and (Test-Path -LiteralPath $p -PathType Leaf)) { Remove-Item -LiteralPath $p -Force }
  $o += (git checkout -- $p 2>&1 | Out-String)
}
$o += (git status --short 2>&1 | Out-String)
$o | Out-File -Encoding utf8 out/restore.log
