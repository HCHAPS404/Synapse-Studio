# Recreates services/core/.venv (regenerable) and runs ruff + pytest.
# Log: out/venv.log, ends with "VENV DONE".
$root = Split-Path -Parent $PSScriptRoot
$core = Join-Path $root 'services/core'
New-Item -ItemType Directory -Force -Path (Join-Path $root out) | Out-Null
$log = Join-Path $root 'out/venv.log'
"venv $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
Set-Location $core
# Move the damaged environment aside (files may be locked), then create a fresh one.
if (Test-Path .venv) { Rename-Item .venv ".venv-broken-$(Get-Date -Format 'yyyyMMdd-HHmmss')" -ErrorAction SilentlyContinue }
$env:UV_LINK_MODE = 'copy'
$s = uv sync --no-progress 2>&1 | Out-String
"==== uv sync (exit $LASTEXITCODE) ====`n$s" | Out-File -Append -Encoding utf8 $log
$r = uv run --no-progress ruff check . 2>&1 | Out-String
"==== ruff (exit $LASTEXITCODE) ====`n$r" | Out-File -Append -Encoding utf8 $log
$f = uv run --no-progress ruff format --check . 2>&1 | Out-String
"==== ruff format (exit $LASTEXITCODE) ====`n$f" | Out-File -Append -Encoding utf8 $log
$p = uv run --no-progress pytest -q -p no:cacheprovider 2>&1 | Out-String
"==== pytest (exit $LASTEXITCODE) ====`n$p" | Out-File -Append -Encoding utf8 $log
"VENV DONE" | Out-File -Append -Encoding utf8 $log
