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

| Arduino Nano 33 IoT | COM8 (USB, Arduino VID 2341 / PID 8057; direct, not a bridge) | `nano_33_iot` / SAMD21G18A | ✅ | ✅ `app.elf`+`app.bin`, 1592 B, reset vector 0x00002429 (above the SAM-BA bootloader at 0x2000), stack 0x20008000 — the SAMD21 Cortex-M0+ port compiles and boots cleanly | ⏳ | Flash with bossac (`NEURON_BOSSAC`, offset 0x2000) over COM8; bossac not installed yet. Works: GPIO + 1 ms tick. UART TX and edge interrupts are LEFT OUT in this version (SERCOM/EIC need GCLK). LED PA17 to confirm. Runs at 8 MHz (48 MHz DFLL later). |

| Arduino Nano 33 BLE Sense Rev2 | COM10 (USB, Arduino VID 2341 / PID 805A; direct) | `nano_33_ble_sense` / nRF52840 | ✅ | ✅ `app.elf`+`app.bin`+**`app.uf2`**, 2024 B, reset vector 0x000264b9 (above the UF2 bootloader at 0x26000), stack 0x20040000, UF2 family 0xADA52840 — the nRF52840 Cortex-M4F port compiles and boots cleanly | ⏳ | **Easiest of the three new boards to flash: no tool needed.** Double-tap RESET → the NANO33BLE drive appears → copy `app.uf2`. Works: GPIO, 1 ms tick, UARTE0 TX, GPIOTE edge. LED P0.13. Serial1 pins P1.03/P1.10 to confirm. |

| TI EK-TM4C123GXL (Tiva C LaunchPad) | COM11 (USB, VID 1CBE / PID 00FD; on-board ICDI debugger) | `ek_tm4c123gxl` / TM4C123GH6PM | ✅ | ✅ `app.elf`+`app.bin`, 2168 B, reset vector 0x0000061d (flash at 0x00000000, Thumb bit set), stack 0x20008000 — the TM4C123 Cortex-M4F port compiles and boots cleanly with arm-none-eabi-gcc | ⏳ | Flash with lm4flash over the on-board ICDI (`NEURON_LM4FLASH`), or OpenOCD `board/ek-tm4c123gxl.cfg`; lm4flash not installed yet. Works: GPIO + input pull-up + 1 ms SysTick tick. UART0 TX and per-port edge interrupts LEFT OUT until the board session (TM4C GPIO interrupt is per-port, not per-line); ADC/PWM left out. LED PF1 (red). Runs at 16 MHz PIOSC (80 MHz PLL later). |

| Arduino Nano (classic, CH340 clone) | USB CH340 (VID 1A86 / PID 7523), no COM yet (driver missing) | `arduino_nano` / ATmega328P (new board, same chip as the UNO R3) | ✅ (model) | ✅ C backend with avr-gcc 7.3: `app.hex` 2952 B (9% flash); native backend: 642 B — the native backend now accepts any ATmega328P board, not only the UNO R3 | ⏳ | **Install the CH340 driver first** so a COM port appears; then flash with avrdude at **57600 baud** (the old Nano bootloader, set in the board file). A6/A7 are analog-only. This is the easiest board to flash once the driver is in (avrdude already installed). |

| NXP/Freescale FRDM-KL43Z | USB OpenSDA (P&E Micro, VID 1357 / PID 0089; mounts an MSD mass-storage drive) | `frdm_kl43z` / MKL43Z256VLH4 (Kinetis KL43, Cortex-M0+) | ✅ | ✅ `app.elf`+`app.bin`, 2536 B, reset vector 0x00000789 (flash at 0x00000000, Thumb bit set), stack 0x20006000, flash-config field at 0x400 unsecured (0x40E=0xFF FOPT, 0x40F=0xFE FSEC) — the Kinetis KL43 Cortex-M0+ port compiles and boots cleanly with arm-none-eabi-gcc | ⏳ | **No tool needed:** copy `app.bin` to the OpenSDA MSD drive (tool `msd`; `--drive`) and the bootloader programs it. Works: GPIO + input pull-up + 1 ms SysTick tick. LPUART0 TX and PORT edge interrupts LEFT OUT (LPUART clock-source and per-port NVIC wiring need the board); ADC/PWM left out. LED PTE31 (red). RAM straddles 0x20000000 (region at 0x1FFFE000). Runs at 4 MHz FIRC (48 MHz later). |

## Pending test batch (end of session)

- Install a flasher: `probe-rs` (`cargo install probe-rs-tools`) and/or st-flash/OpenOCD.
- UNO Q: resolve the STM32U585 flash route (likely via the board's Linux side).
- Flash the Tier 1 boards (UNO R3, Mega, Pico, Pico 2 W) and watch the LED.
- Confirm the placeholder LED/UART pins on each new board.
- Pending CLIs / toolchains: ESP-IDF for the ESP32-S3; bossac for the Nano 33 IoT;
  lm4flash (or OpenOCD) for the EK-TM4C123GXL.

## Measurements to take (replace the generic numbers in Phases D/E)

PLC input filter and scan, robot command latency, valve switching time, real link
transport and jitter per medium. Then write a timing profile TOML and run
`neuron cell check --profile <file>` (E6.3) and move manifests from `generic` to `measured`.

## Session summary (2026-11-23)

Seven devices verified in software this session; none flashed yet (the flash batch waits
for its tools). Six distinct MCU families plus an FPGA were validated by compiling real
firmware with arm-none-eabi-gcc / avr-gcc, or (FPGA) by emitting Verilog + XDC.

| Device | Model | Status |
|---|---|---|
| Arduino UNO Q | STM32U585 (Cortex-M33) | verified + firmware; flash via the board's Linux side (SWD), to resolve |
| Nexys A7-100T | Artix-7 xc7a100t | board added; blink + VGA Verilog/XDC; bitstream needs an FPGA toolchain |
| Arduino Nano 33 IoT | SAMD21 (Cortex-M0+) | verified + firmware; flash with bossac |
| Arduino Nano 33 BLE Sense Rev2 | nRF52840 (Cortex-M4F) | verified + firmware + UF2; flash by copying the UF2 |
| EK-TM4C123GXL (Tiva C) | TM4C123 (Cortex-M4F) | board added + firmware; flash with lm4flash (ICDI) |
| Arduino Nano (classic) | ATmega328P (AVR) | board added + firmware; needs the CH340 driver, then avrdude 57600 |
| FRDM-KL43Z | Kinetis KL43 (Cortex-M0+) | board added + firmware (flash-config unsecured); flash by copying app.bin to the OpenSDA drive |

## Still to acquire / schedule (hardware, future sessions)

- **Festo lab / PLCs (session I5):** ST/SCL import into the vendor tools, Micro-Kernel,
  Modbus/S7 links. The software path (Phase C emitters, D cell links) is done.
- **Epson robot arm (session I6):** the SPEL+ emitter and slave interpreter, workspace and
  zone checks at reduced speed. Software done; the vendor dialect is to confirm on the arm.
- **Hexacopter with a Pixhawk 4 (session I9):** not yet acquired. Inventory and bench only
  (propellers off). No MAVLink adapter, drone manifest or spec exists yet — it needs its
  own spec first (link, manifest, safety: arming, geofence, failsafe). Do not claim flight.
- **More boards:** UNO R3, Mega 2560, Pico, Pico 2 W, ESP32-S3 are already supported and
  only need connecting and verifying. Nucleo N6 (STM32N657) is `described` only (no runtime
  port yet). A Spartan FPGA would need its own `.fpga.toml` (give the exact part).
