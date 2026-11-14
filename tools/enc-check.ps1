# Prints the first bytes of every file in a folder to find encoding problems. Result: out/enc.txt
param([string]$Dir = 'examples')
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$o = foreach ($f in Get-ChildItem $Dir -File) {
  $b = [System.IO.File]::ReadAllBytes($f.FullName)
  $bad = 0
  try { $null = (New-Object System.Text.UTF8Encoding($false, $true)).GetString($b) } catch { $bad = 1 }
  "{0} len={1} head={2} invalidUtf8={3}" -f $f.Name, $b.Length, (($b[0..3] | ForEach-Object { $_.ToString('x2') }) -join ' '), $bad
}
$o | Out-File -Encoding utf8 out/enc.txt
