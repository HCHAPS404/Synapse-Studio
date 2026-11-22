# Runs locally what CI runs, then optionally commits and pushes.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\phase-check.ps1                  # checks only
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\phase-check.ps1 -MessageFile m.txt  # checks, then ship
# Steps: build the CLI; `neuron check`, `fmt --check` and `test` on examples, every corpus folder
# and lib (blocking, as in CI); smoke runs of `build --native`, `emit-verilog` and `targets`,
# `emit` (st, spel, python, kernel) and `import st` of the emitted ST, `cell check` of the four corpus cells, `cell build`, `cell dossier` and `cell sim` of x02 (informational); then tools\ship.ps1 (fmt, clippy, all tests, commit -s, push).
# Takes 6-10 minutes with a warm build cache, 20-25 minutes after `cargo clean`.
# Log: out/phase-check.log.
param([string]$Message = '', [string]$MessageFile = '')
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$tc = Join-Path $env:USERPROFILE 'toolchains'
foreach ($d in 'w64devkit\bin', 'avr-gcc\bin', 'avrdude\bin', 'arm-none-eabi\bin') {
  if (Test-Path "$tc\$d") { $env:PATH = "$tc\$d;$env:PATH" }
}
$env:NEURON_CC = 'gcc'; $env:NEURON_AVR_GCC = 'avr-gcc'; $env:NEURON_AVRDUDE = 'avrdude'
$env:NEURON_ARM_GCC = 'arm-none-eabi-gcc'; $env:NEURON_REQUIRE_TOOLCHAIN = '1'; $env:NEURON_REQUIRE_ARM = '1'
$env:CARGO_TERM_COLOR = 'never'
$log = Join-Path $root 'out\phase-check.log'
"phase-check $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
function L($m) { Write-Host $m; $m | Out-File -Append -Encoding utf8 $log }
$t0 = Get-Date

L '[1/4] Building the CLI...'
cmd /c "cargo build -q -p neuron-cli > out\phase-check-build.log 2>&1"
if ($LASTEXITCODE -ne 0) { L 'PHASE-CHECK DONE: stopped (the CLI does not build, see out\phase-check-build.log)'; exit 1 }
$neuron = Join-Path $root 'target\debug\neuron.exe'

L '[2/4] CLI checks that CI runs (blocking)...'
$steps = @(@('check', 'examples'), @('fmt', '--check', 'examples'), @('test', 'examples'))
foreach ($d in Get-ChildItem examples\corpus -Directory) {
  $p = "examples/corpus/$($d.Name)"
  $steps += , @('check', $p); $steps += , @('fmt', '--check', $p); $steps += , @('test', $p)
}
$steps += , @('check', 'lib'); $steps += , @('fmt', '--check', 'lib'); $steps += , @('test', 'lib')
$bad = 0
foreach ($s in $steps) {
  $out = cmd /c "`"$neuron`" $($s -join ' ') 2>&1" | Out-String
  if ($LASTEXITCODE -ne 0) { $bad++; L "  FAIL: neuron $($s -join ' ')"; $out | Out-File -Append -Encoding utf8 $log }
}
L "  $($steps.Count) steps, $bad failed"
if ($bad -gt 0) { L 'PHASE-CHECK DONE: stopped (CLI checks failed, details above in out\phase-check.log)'; exit 1 }

L '[3/4] Smoke runs of the Phase B commands (informational)...'
$smoke = @(
  @('build', 'examples/corpus/mcu/m01_blink.neuron', '--native', '--out', 'out/native'),
  @('emit-verilog', 'examples/corpus/hdl/d01_counter.neuron', '--board', 'artix7_35t_board', '-o', 'out/hdl'),
  @('targets'),
  @('emit', 'st', 'examples/corpus/plc/p06_pump_with_levels.neuron', '-o', 'out/st'),
  @('emit', 'spel', 'examples/corpus/robot/r02_palletize.neuron', '-o', 'out/spel'),
  @('emit', 'python', 'examples/corpus/host/h01_gateway_counter.neuron', '-o', 'out/python'),
  @('emit', 'kernel', 'examples/corpus/mcu/m06_traffic_light.neuron', '-o', 'out/kernel'),
  @('import', 'st', 'out/st/p06_pump_with_levels.st', '-o', 'out/imported/p06_pump_with_levels.neuron'),
  @('check', 'out/imported/p06_pump_with_levels.neuron'),
  @('cell', 'check', 'examples/corpus/cell/x01_two_devices.neuron'),
  @('cell', 'check', 'examples/corpus/cell/x02_robot_zone.neuron'),
  @('cell', 'check', 'examples/corpus/cell/x03_cycle_sequence.neuron'),
  @('cell', 'check', 'examples/corpus/cell/x04_event_from_fpga.neuron'),
  @('cell', 'build', 'examples/corpus/cell/x02_robot_zone.neuron', '-o', 'out/cell/x02'),
  @('cell', 'dossier', 'examples/corpus/cell/x02_robot_zone.neuron', '-o', 'out/cell/x02', '--for', '200ms'),
  @('cell', 'sim', 'examples/corpus/cell/x02_robot_zone.neuron', '--for', '200ms', '--cut', 'l0@100ms', '--replay', 'out/cell/x02/replay.json')
)
foreach ($s in $smoke) {
  $out = cmd /c "`"$neuron`" $($s -join ' ') 2>&1" | Out-String
  $r = if ($LASTEXITCODE -eq 0) { 'ok  ' } else { 'FAIL' }
  L "  $r neuron $($s -join ' ')"
  if ($r -eq 'FAIL') { $out | Out-File -Append -Encoding utf8 $log }
}

if ($MessageFile) { $Message = Get-Content -Raw $MessageFile }
if (-not $Message) { L "PHASE-CHECK DONE: checks passed in $([int]((Get-Date) - $t0).TotalMinutes) min (no commit requested)"; exit 0 }

L '[4/4] fmt + clippy + all tests, then commit and push (tools\ship.ps1)...'
powershell -NoProfile -ExecutionPolicy Bypass -File tools\ship.ps1 -Message $Message
L ((Get-Content out\ship.log -Tail 3) | Out-String)
L "develop local=$(Get-Content .git\refs\heads\develop -ErrorAction SilentlyContinue) origin=$(Get-Content .git\refs\remotes\origin\develop -ErrorAction SilentlyContinue)"
L "PHASE-CHECK DONE in $([int]((Get-Date) - $t0).TotalMinutes) min. CI: https://github.com/HCHAPS404/Synapse-Studio/actions"
