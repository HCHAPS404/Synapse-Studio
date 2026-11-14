# Compares the content of every tracked file with the committed blob (not only timestamps),
# to detect silent corruption of the drive. Log: out/verify-worktree.log
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$o = @("verify-worktree $(Get-Date -Format o)")
$bad = 0; $n = 0
$entries = git ls-files -s 2>$null
foreach ($e in $entries) {
  if ($e -match '^(\d+)\s+([0-9a-f]{40})\s+\d+\t(.+)$') {
    $mode = $Matches[1]; $blob = $Matches[2]; $path = $Matches[3]
    if ($mode -eq '160000') { continue }
    $n++
    if (-not (Test-Path -LiteralPath $path)) { $o += "MISSING $path"; $bad++; continue }
    $h = git hash-object -- $path 2>$null
    if ($h -ne $blob) { $o += "DIFFERENT $path"; $bad++ }
  }
}
$o += "checked=$n different_or_missing=$bad"
$o += "VERIFY DONE"
$o | Out-File -Encoding utf8 out/verify-worktree.log
