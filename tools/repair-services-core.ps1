# Tries to repair the damaged directory entry services/core (all its files are tracked in git).
# Log: out/repair-core.log
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$o = @("repair $(Get-Date -Format o)")
$o += "== list services"
$o += (Get-ChildItem services -Force -ErrorAction Continue 2>&1 | Out-String)
$o += "== test-path"
$o += "services/core exists: $(Test-Path 'services/core')"
$o += "== rmdir via cmd"
$full = (Resolve-Path 'services').Path + '\core'
$o += (cmd /c "rmdir /s /q `"\\?\$full`"" 2>&1 | Out-String)
$o += "== rename attempt"
try { Rename-Item -LiteralPath "\\?\$full" -NewName "core-broken-20261114" -ErrorAction Stop; $o += "renamed" } catch { $o += "rename failed: $($_.Exception.Message)" }
$o += "== try mkdir"
try { New-Item -ItemType Directory -Path 'services/core' -ErrorAction Stop | Out-Null; $o += "mkdir ok" } catch { $o += "mkdir failed: $($_.Exception.Message)" }
$o += "== list services after"
$o += (Get-ChildItem services -Force -ErrorAction Continue 2>&1 | Out-String)
$o | Out-File -Encoding utf8 out/repair-core.log
