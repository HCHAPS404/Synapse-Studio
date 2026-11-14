# Lists C/embedded toolchains available on this machine. Log: out/toolchains.log
$root = Split-Path -Parent $PSScriptRoot
New-Item -ItemType Directory -Force -Path (Join-Path $root out) | Out-Null
$log = Join-Path $root 'out/toolchains.log'
"toolchains $(Get-Date -Format o)" | Out-File -Encoding utf8 $log
$tools = 'gcc','clang','cc','avr-gcc','avrdude','arm-none-eabi-gcc','riscv32-esp-elf-gcc','xtensa-esp32-elf-gcc','msp430-elf-gcc','openocd','probe-rs','esptool','pio','make','wsl'
foreach ($t in $tools) {
  $c = Get-Command $t -ErrorAction SilentlyContinue
  if ($c) {
    $v = (& $t --version 2>&1 | Select-Object -First 1) -as [string]
    "$t : $($c.Source) : $v" | Out-File -Append -Encoding utf8 $log
  } else { "$t : MISSING" | Out-File -Append -Encoding utf8 $log }
}
# Common install locations not always on PATH
$extra = @(
  "$env:USERPROFILE\.platformio\packages",
  "$env:LOCALAPPDATA\Arduino15\packages",
  "C:\Program Files (x86)\Arduino\hardware\tools\avr\bin",
  "C:\msys64\ucrt64\bin", "C:\msys64\mingw64\bin",
  "C:\Program Files\LLVM\bin"
)
foreach ($p in $extra) { "path $p exists: $(Test-Path $p)" | Out-File -Append -Encoding utf8 $log }
if (Test-Path "$env:LOCALAPPDATA\Arduino15\packages") {
  Get-ChildItem "$env:LOCALAPPDATA\Arduino15\packages" -Recurse -Filter '*gcc.exe' -ErrorAction SilentlyContinue | Select-Object -First 10 -ExpandProperty FullName | Out-File -Append -Encoding utf8 $log
}
if (Test-Path "$env:USERPROFILE\.platformio\packages") {
  Get-ChildItem "$env:USERPROFILE\.platformio\packages" -Recurse -Filter '*gcc.exe' -ErrorAction SilentlyContinue | Select-Object -First 10 -ExpandProperty FullName | Out-File -Append -Encoding utf8 $log
}
"done" | Out-File -Append -Encoding utf8 $log
