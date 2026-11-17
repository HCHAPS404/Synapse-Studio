# Runs one command detached from the editor terminal and logs it.
#   Start-Process powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','tools\job.ps1','-Cmd','cargo test -p x','-Log','x.log' -WindowStyle Hidden
# The log (out/<Log>) ends with "JOB END exit=<code>". The C toolchains of
# tools/install-toolchains.ps1 are put on PATH first.
param([Parameter(Mandatory = $true)][string]$Cmd, [string]$Log = 'job.log')
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path out | Out-Null
$tc = Join-Path $env:USERPROFILE 'toolchains'
foreach ($d in 'w64devkit/bin', 'avr-gcc/bin', 'avrdude/bin', 'arm-none-eabi/bin') {
  $p = Join-Path $tc $d
  if (Test-Path $p) { $env:PATH = "$p;$env:PATH" }
}
$env:NEURON_CC = 'gcc'; $env:NEURON_AVR_GCC = 'avr-gcc'; $env:NEURON_AVRDUDE = 'avrdude'
if (Test-Path (Join-Path $tc 'arm-none-eabi/bin')) { $env:NEURON_ARM_GCC = 'arm-none-eabi-gcc'; $env:NEURON_REQUIRE_ARM = '1' }
$env:NEURON_REQUIRE_TOOLCHAIN = '1'
$env:CARGO_TERM_COLOR = 'never'
$env:CARGO_TERM_PROGRESS_WHEN = 'never'
$path = Join-Path $root "out/$Log"
"JOB START $(Get-Date -Format o): $Cmd" | Out-File -Encoding utf8 $path
# Parentheses make the redirection cover every command of an `a && b` chain.
cmd /c "($Cmd) >> `"$path`" 2>&1"
"JOB END exit=$LASTEXITCODE $(Get-Date -Format o)" | Out-File -Append -Encoding utf8 $path
