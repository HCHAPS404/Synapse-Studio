# Lab inventory (session I1)

What each device is, how the system sees it, and how far SYNAPSE got with it. Filled in
live during the session. "Verified" = detected + model loads + `neuron check`/`emit-c`
work. "Firmware built" = compiled with the real toolchain to an image. "Flashed" = written
to the board and the program observed running. Timing numbers to measure here replace the
generic placeholders in Phases D and E.

Status legend: ✅ done · ⏳ pending (this session) · ⚠️ blocked, see note.

| Device | Port / interface | Model (board / chip) | Verified | Firmware built | Flashed | Notes |
|---|---|---|---|---|---|---|
| Arduino UNO Q | COM6 (USB, Arduino VID 2341 / PID 0078; the Qualcomm Linux side) | `uno_q` / STM32U585 | ✅ | ✅ `app.elf`+`app.bin`, 2412 B, reset vector 0x08000641, stack 0x200C0000 — the STM32U5 port compiles and boots cleanly with arm-none-eabi-gcc 15.2.1 | ⏳ | SWD flashing not done: no st-flash/openocd/probe-rs installed, and the UNO Q programs its STM32 through the on-board Linux side, not an external SWD port. LED PA5 and USART1 TX PA9 still to confirm. Linux side (App Lab, user projects) is untouched. |

| Digilent Nexys A7-100T | USB (Digilent FTDI FT2232: JTAG + UART; driver showed "Error" — Adept/Vivado not installed) | `nexys_a7_100t` / xc7a100tcsg324-1 | ✅ added the board to the model (new `.fpga.toml`, registered in `neuron-codegen-hdl`) | ✅ Verilog + XDC generated for a 1 Hz blink: clock E3 (100 MHz, 10 ns), LED0 H17, LVCMOS33 (`out/nexys/top.v`, `top.xdc`) | ⏳ | The existing `artix7_35t_board` did NOT fit: it is a 35T in the CPG236 package, this is a 100T in CSG324 (different pinout). No FPGA tools installed (iverilog, yosys, Vivado, openFPGALoader), so synthesis and the bitstream are pending. Example `examples/nexys_a7_blink.neuron`. Pins from the public Digilent pinout; confirm against the board's reference XDC before flashing. |

## Pending test batch (end of session)

- Install a flasher: `probe-rs` (`cargo install probe-rs-tools`) and/or st-flash/OpenOCD.
- UNO Q: resolve the STM32U585 flash route (likely via the board's Linux side).
- Flash the Tier 1 boards (UNO R3, Mega, Pico, Pico 2 W) and watch the LED.
- Confirm the placeholder LED/UART pins on each new board.
- Pending CLIs / toolchains: ESP-IDF for the ESP32-S3; bossac for the Nano 33 IoT.

## Measurements to take (replace the generic numbers in Phases D/E)

PLC input filter and scan, robot command latency, valve switching time, real link
transport and jitter per medium. Then write a timing profile TOML and run
`neuron cell check --profile <file>` (E6.3) and move manifests from `generic` to `measured`.
