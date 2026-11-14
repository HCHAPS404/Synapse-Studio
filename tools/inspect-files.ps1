# Inspects files: size, first bytes, valid UTF-8, and the hash git computes versus the committed blob.
# Usage: powershell -File tools/inspect-files.ps1 -Paths a,b   Log: out/inspect.log
param([string[]]$Paths)
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$o = @("inspect $(Get-Date -Format o)")
foreach ($p in ($Paths | ForEach-Object { $_ -split ',' })) {
  $b = [System.IO.File]::ReadAllBytes($p)
  $bad = 0
  try { $null = (New-Object System.Text.UTF8Encoding($false, $true)).GetString($b) } catch { $bad = 1 }
  $crlf = ([regex]::Matches([System.Text.Encoding]::Latin1.GetString($b), "`r`n")).Count
  $h = git hash-object -- $p
  $blob = (git ls-files -s -- $p) -replace '^\d+\s+([0-9a-f]{40}).*$', '$1'
  $o += ("{0} len={1} head={2} invalidUtf8={3} crlf={4} hash={5} blob={6}" -f $p, $b.Length, (($b[0..3] | ForEach-Object { $_.ToString('x2') }) -join ' '), $bad, $crlf, $h, $blob)
}
$o | Out-File -Encoding utf8 out/inspect.log
