<#
.SYNOPSIS
  Installs the C toolchains used by Phase B into a user folder, outside the repository.
.DESCRIPTION
  - w64devkit (GCC for the host), from the skeeto/w64devkit GitHub release.
  - avr-gcc and avrdude, from the official Arduino package index.
  Every download is verified against a published SHA-256 before it is extracted. Nothing is
  installed system-wide and the PATH is not modified: tools/toolchain-env.ps1 prints the
  folders to add for one session. Log: out/toolchain.log
  Usage: powershell -ExecutionPolicy Bypass -File tools\install-toolchains.ps1 [-Root <dir>]
#>
param([string]$Root = (Join-Path $env:USERPROFILE 'toolchains'))

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = Split-Path -Parent $PSScriptRoot
New-Item -ItemType Directory -Force (Join-Path $repo 'out') | Out-Null
$log = Join-Path $repo 'out/toolchain.log'
"install-toolchains $(Get-Date -Format o) root=$Root" | Set-Content $log
function L($m) { $m | Add-Content $log }

function Get-Verified([string]$url, [string]$file, [string]$sha256) {
  L "download $url"
  Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $file
  $got = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant()
  if ($got -ne $sha256.ToLowerInvariant()) {
    Remove-Item $file -Force
    throw "SHA-256 mismatch for $url (expected $sha256, got $got)"
  }
  L "sha256 ok $got"
}

New-Item -ItemType Directory -Force $Root | Out-Null
$dl = Join-Path $Root '_downloads'
New-Item -ItemType Directory -Force $dl | Out-Null

try {
  # ---- host GCC ------------------------------------------------------------------
  $w64 = Join-Path $Root 'w64devkit'
  if (-not (Test-Path (Join-Path $w64 'bin/gcc.exe'))) {
    $exe = Join-Path $dl 'w64devkit-x64-2.11.0.7z.exe'
    Get-Verified 'https://github.com/skeeto/w64devkit/releases/download/v2.11.0/w64devkit-x64-2.11.0.7z.exe' `
      $exe '54c2bb9962e8a30c8270e34c5efc3d5fee579f53087fa7d435b95fbba2af7b2b'
    & $exe "-o$Root" -y | Out-Null
    L "extracted w64devkit"
  } else { L "w64devkit already present" }

  # ---- AVR tools from the Arduino package index -------------------------------------
  $indexPath = Join-Path $dl 'package_index.json'
  Invoke-WebRequest -UseBasicParsing -Uri 'https://downloads.arduino.cc/packages/package_index.json' -OutFile $indexPath
  $index = Get-Content $indexPath -Raw | ConvertFrom-Json
  $arduino = $index.packages | Where-Object { $_.name -eq 'arduino' } | Select-Object -First 1
  foreach ($tool in 'avr-gcc', 'avrdude') {
    $dest = Join-Path $Root $tool
    if (Test-Path $dest) { L "$tool already present"; continue }
    # Pinned versions (the index lists newest first, so "last" would be the oldest).
    $pinned = @{ 'avr-gcc' = '7.3.0-atmel3.6.1-arduino7'; 'avrdude' = '6.3.0-arduino17' }
    $entry = $arduino.tools | Where-Object { $_.name -eq $tool -and $_.version -eq $pinned[$tool] } | Select-Object -First 1
    if (-not $entry) { throw "$tool $($pinned[$tool]) is not in the Arduino index" }
    $sys = $entry.systems | Where-Object { $_.host -match 'mingw32' } | Select-Object -First 1
    if (-not $sys) { throw "no Windows build of $tool in the Arduino index" }
    $sha = ($sys.checksum -replace '^SHA-256:', '')
    $zip = Join-Path $dl $sys.archiveFileName
    Get-Verified $sys.url $zip $sha
    $tmp = Join-Path $dl "x_$tool"
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $tmp
    # The archive holds one top-level folder (the tool). Move its content to <Root>/<tool>.
    $inner = Get-ChildItem $tmp | Select-Object -First 1
    Move-Item $inner.FullName $dest
    L "extracted $tool $($entry.version)"
  }
  # ---- Arm GCC (xPack) for the RP2040, RP2350 and STM32 ports ----------------------------
  $arm = Join-Path $Root 'arm-none-eabi'
  if (-not (Test-Path (Join-Path $arm 'bin/arm-none-eabi-gcc.exe'))) {
    $ver = '15.2.1-1.1'
    $zip = Join-Path $dl "xpack-arm-none-eabi-gcc-$ver-win32-x64.zip"
    # Digest published with the GitHub release asset.
    Get-Verified "https://github.com/xpack-dev-tools/arm-none-eabi-gcc-xpack/releases/download/v$ver/xpack-arm-none-eabi-gcc-$ver-win32-x64.zip" `
      $zip 'bae6a3d1667697ce750c3b13d6d26d80973ecedc2cc87bf04869e83447fd93ea'
    $tmp = Join-Path $dl 'x_arm'
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $tmp
    $inner = Get-ChildItem $tmp | Select-Object -First 1
    Move-Item $inner.FullName $arm
    L "extracted arm-none-eabi-gcc $ver"
  } else { L "arm-none-eabi-gcc already present" }
  L "DONE"
} catch {
  L "FAILED: $($_.Exception.Message)"
  exit 1
}
