# Lists build and test processes to find stuck jobs. Output: out/ps.txt. -Kill stops them.
param([switch]$Kill)
Set-Location (Split-Path -Parent $PSScriptRoot)
New-Item -ItemType Directory -Force -Path out | Out-Null
$names = 'cargo', 'rustc', 'clippy-driver', 'neuron', 'a4_names*', 'a5_types*', 'a6_rules*', 'a7_time*', 'a8_safety*', 'neuron_sema*', 'rustdoc'
$p = Get-Process -Name $names -ErrorAction SilentlyContinue
$o = $p | Select-Object Id, ProcessName, StartTime, CPU | Format-Table -AutoSize | Out-String
$o | Out-File -Encoding utf8 out/ps.txt
if ($Kill -and $p) { $p | Stop-Process -Force -ErrorAction SilentlyContinue; "killed" | Out-File -Append -Encoding utf8 out/ps.txt }
