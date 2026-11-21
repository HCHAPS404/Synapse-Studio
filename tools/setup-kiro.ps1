# Configures Kiro so the agent can work without approval prompts, then validates the
# configuration and the repository. Run it yourself: Kiro never lets the agent edit its
# own permission files.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-kiro.ps1          # configure + quick repo check
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-kiro.ps1 -Full    # configure + full repo check (10-25 min)
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\setup-kiro.ps1 -CheckOnly
# What it changes (each file is backed up first, next to itself, with a date suffix):
#   1. %USERPROFILE%\.kiro\settings\permissions.yaml : allow everything, deny a few destructive commands
#   2. %APPDATA%\Kiro\User\settings.json             : "kiroAgent.agentAutonomy": "Autopilot"
# Restart Kiro afterwards. Log: out\setup-kiro.log
param([switch]$Full, [switch]$CheckOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$log = Join-Path $root 'out\setup-kiro.log'
"setup-kiro $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
function L($m) { Write-Host $m; "$m" | Out-File -Append -Encoding utf8 $log }
$results = [ordered]@{}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$utf8 = New-Object System.Text.UTF8Encoding($false)

$permDir = Join-Path $env:USERPROFILE '.kiro\settings'
$perm = Join-Path $permDir 'permissions.yaml'
$settings = Join-Path $env:APPDATA 'Kiro\User\settings.json'

$yaml = @'
# Written by tools/setup-kiro.ps1 (Synapse Studio). Allows the agent to work without
# prompts; the deny rules below always win over the allow.
rules:
  - capability: all
    effect: allow
  - capability: shell
    match:
      - "git push --force*"
      - "git push -f*"
      - "git reset --hard*"
      - "git clean -fd*"
      - "rm -rf *"
      - "Remove-Item -Recurse*"
      - "format *"
    effect: deny
  - capability: fs_write
    match:
      - "**/.env"
      - "**/.env.*"
    effect: deny
'@

if (-not $CheckOnly) {
  # 1. Permissions
  L '== 1. permissions.yaml'
  New-Item -ItemType Directory -Force -Path $permDir | Out-Null
  if (Test-Path $perm) {
    Copy-Item $perm "$perm.bak-$stamp"
    L "   backup: $perm.bak-$stamp"
  }
  [System.IO.File]::WriteAllText($perm, $yaml, $utf8)
  L "   written: $perm"

  # 2. Autonomy (settings.json may contain comments, so it is edited as text, not parsed)
  L '== 2. Agent Autonomy = Autopilot'
  if (-not (Test-Path (Split-Path $settings))) {
    L "   Kiro user folder not found: $(Split-Path $settings). Set it by hand: Settings > Agent > Agent Autonomy > Autopilot"
  } else {
    $line = '"kiroAgent.agentAutonomy": "Autopilot"'
    if (Test-Path $settings) {
      Copy-Item $settings "$settings.bak-$stamp"
      L "   backup: $settings.bak-$stamp"
      $raw = [System.IO.File]::ReadAllText($settings)
      if ($raw -match '"kiroAgent\.agentAutonomy"\s*:\s*"[^"]*"') {
        $raw = [regex]::Replace($raw, '"kiroAgent\.agentAutonomy"\s*:\s*"[^"]*"', $line)
      } elseif ($raw.Trim() -match '^\{\s*\}$' -or $raw.Trim() -eq '') {
        $raw = "{`n  $line`n}`n"
      } else {
        $i = $raw.IndexOf('{')
        if ($i -lt 0) { throw "settings.json has no opening brace; fix it by hand: $settings" }
        $raw = $raw.Substring(0, $i + 1) + "`n  $line," + $raw.Substring($i + 1)
      }
      [System.IO.File]::WriteAllText($settings, $raw, $utf8)
    } else {
      [System.IO.File]::WriteAllText($settings, "{`n  $line`n}`n", $utf8)
    }
    L "   written: $settings"
  }
}

# 3. Validate the configuration
L '== 3. validate Kiro configuration'
$p = if (Test-Path $perm) { [System.IO.File]::ReadAllText($perm) } else { '' }
$results['permissions.yaml allows all'] = if ($p -match 'capability:\s*all' -and $p -match 'effect:\s*allow') { 'OK' } else { "MISSING ($perm)" }
$results['permissions.yaml keeps the deny rules'] = if ($p -match 'git push --force' -and $p -match 'effect:\s*deny') { 'OK' } else { 'MISSING' }
$s = if (Test-Path $settings) { [System.IO.File]::ReadAllText($settings) } else { '' }
$results['Agent Autonomy = Autopilot'] = if ($s -match '"kiroAgent\.agentAutonomy"\s*:\s*"Autopilot"') { 'OK' } else { 'NOT SET (Settings > Agent > Agent Autonomy)' }
$results['Workspace trusted'] = 'CHECK BY HAND (Kiro asks "Trust this workspace?" for C:\dev\Synapse-Studio)'

# 4. Validate the repository and the tools the agent uses
L '== 4. validate tools'
foreach ($t in @('git', 'cargo', 'gh', 'python')) {
  $c = Get-Command $t -ErrorAction SilentlyContinue
  $results["tool $t"] = if ($c) { 'OK' } else { 'NOT FOUND on PATH' }
}
$tc = Join-Path $env:USERPROFILE 'toolchains'
$results['toolchains folder'] = if (Test-Path "$tc\w64devkit\bin\gcc.exe") { 'OK' } else { "gcc not found under $tc\w64devkit" }
$job = Get-Content tools\job.ps1 -Raw
$results['job.ps1 limits cargo to 2 jobs (memory)'] = if ($job -match 'CARGO_BUILD_JOBS') { 'OK' } else { 'MISSING' }

L '== 5. validate repository'
$diag = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools\diagnose.ps1')
if (-not $Full) { $diag += '-Quick' }
$env:CARGO_BUILD_JOBS = '2'
& powershell @diag | Out-Null
$d = Get-Content out\diagnose.log -ErrorAction SilentlyContinue
$sum = $d | Select-String -Pattern '^(ALL GOOD|NOT ALL GOOD.*)$' | Select-Object -Last 1
$results['repository (tools\diagnose.ps1' + $(if ($Full) { ' full' } else { ' -Quick' }) + ')'] = if ($sum) { $sum.Line } else { 'no result, see out\diagnose.log' }
$d | Select-String -Pattern 'ATTENTION|FAIL|MISSING' | ForEach-Object { L "   $($_.Line)" }

L ''
L '================ SUMMARY ================'
foreach ($k in $results.Keys) { L ("{0,-48} {1}" -f $k, $results[$k]) }
L ''
L 'Next: restart Kiro, trust the workspace if asked, then write "sigue" in the chat.'
L 'To undo: copy the .bak-<date> files back over permissions.yaml and settings.json.'
L 'SETUP-KIRO DONE'
