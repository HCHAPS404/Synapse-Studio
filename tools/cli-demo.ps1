# Runs `neuron check` on the examples package and on a deliberately wrong program.
# Log: out/cli.log
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$env:CARGO_TERM_COLOR = 'never'
$bad = @"
module demo for mcu
io led: gpio.output @ D13
signal go: bool
fsm Door every 10 ms:
  initial state Closed:
    -> Open when go
  state Open:
    -> Closed when not go
verify:
  never Open
task blink every 1500 us:
  led.toggle()
fn f(a: f32 mm, b: f32 s) -> f32 mm:
  return a + b
"@
[System.IO.File]::WriteAllText((Join-Path (Get-Location) 'out/demo.neuron'), $bad, (New-Object System.Text.UTF8Encoding($false)))
$o = @("cli-demo $(Get-Date -Format o)")
$o += "== check examples --report --cpu-hz 16000000 --ram 2048"
$o += (cargo run --quiet -p neuron-cli -- check examples --report --cpu-hz 16000000 --ram 2048 2>&1 | Out-String)
$o += "== check out/demo.neuron"
$o += (cargo run --quiet -p neuron-cli -- check out/demo.neuron 2>&1 | Out-String)
$o += "exit=$LASTEXITCODE"
$o | Out-File -Encoding utf8 out/cli.log
"CLI DEMO DONE" | Out-File -Append -Encoding utf8 out/cli.log
