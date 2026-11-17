<#
.SYNOPSIS
  Prepends the toolchains installed by install-toolchains.ps1 to PATH for this session.
.DESCRIPTION
  Dot-source it:  . .\tools\toolchain-env.ps1
  Also sets NEURON_CC (host C compiler) and NEURON_AVR_GCC / NEURON_AVRDUDE for the tools.
#>
param([string]$Root = (Join-Path $env:USERPROFILE 'toolchains'))

$dirs = @(
  (Join-Path $Root 'w64devkit/bin'),
  (Join-Path $Root 'avr-gcc/bin'),
  (Join-Path $Root 'avrdude/bin'),
  (Join-Path $Root 'arm-none-eabi/bin')
) | Where-Object { Test-Path $_ }
if ($dirs) { $env:PATH = ($dirs -join ';') + ';' + $env:PATH }
$env:NEURON_CC = 'gcc'
$env:NEURON_AVR_GCC = 'avr-gcc'
$env:NEURON_AVRDUDE = 'avrdude'
$env:NEURON_ARM_GCC = 'arm-none-eabi-gcc'
