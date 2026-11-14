# Fails if any .kiro path is ignored by git.
# Kiroween 2026 rules: the /.kiro directory must be at the repository root and
# must NOT be in .gitignore (it may disqualify the submission).
$ErrorActionPreference = 'Stop'

$root = git rev-parse --show-toplevel
Set-Location $root

$paths = @(
  '.kiro',
  '.kiro/specs/example/requirements.md',
  '.kiro/steering/example.md',
  '.kiro/hooks/example.json',
  '.kiro/agents/example.json',
  '.kiro/settings/mcp.json'
)

$failed = $false
foreach ($p in $paths) {
  git check-ignore -q -- $p
  if ($LASTEXITCODE -eq 0) {
    Write-Error "ERROR: '$p' is ignored by git. Remove the matching rule from .gitignore."
    $failed = $true
  }
}

if (-not (Test-Path '.kiro')) {
  Write-Error "ERROR: /.kiro directory is missing at the repository root."
  $failed = $true
}

if ($failed) { exit 1 }
Write-Output "OK: .kiro is present and not ignored."
exit 0
