# Lab session plan (hardware)

Order devices by how solid the software support is. Lab safety every time: isolated
network, a hardwired E-STOP as the authority (the software E-STOP is supplementary),
optocouplers or level shifters between 3.3 V and 24 V, dry run first, reduced speed for
motion. This tool is a design aid, not a certified safety tool (no IEC 61508 / ISO 13849
claims).

For each board the first three programs are the gate: blink, UART, an interrupt.
Corpus programs: `examples/corpus/mcu/m01_blink.neuron`, `m02_button_toggle.neuron`,
`m11_interrupt_priorities.neuron` (check `--board` support per note below).

## Tier 1 — full support, flash today

| Board | Build/flash command | Tool needed |
|---|---|---|
| Arduino UNO R3 | `neuron flash <f> --port COMx` and `--native` | avrdude (or Arduino IDE) |
| Arduino Mega 2560 | `neuron flash <f> --board arduino_mega_2560 --port COMx` | avrdude |
| Raspberry Pi Pico | hold BOOTSEL, plug in, then `neuron flash <f> --board raspberry_pi_pico --drive E:` | UF2 drive |
| Raspberry Pi Pico 2 W | same, `--board raspberry_pi_pico_2w` | UF2 drive |

## Tier 2 — supported, needs a toolchain installed first

| Board | Command | Install first |
|---|---|---|
| ESP32-S3 DevKit | `neuron flash <f> --board esp32_s3_devkit --port COMx` | ESP-IDF 5.x (the build generates an idf.py project) |

## Tier 3 — NEW ports, built tonight, first run on hardware is this session

These compiled in the model and emit C, but the C ports have never been compiled with
arm-none-eabi-gcc here and never run on a board. Expect to debug. GPIO + the 1 ms tick are
the safe bet on all three; UART and interrupts are best-effort (see each note).

| Board | Chip / port | Flash | Works | Best-effort / to confirm |
|---|---|---|---|---|
| Arduino Nano 33 BLE Sense Rev2 | nRF52840 / nrf52 | double-tap RESET → UF2 drive NANO33BLE | GPIO, tick, UART (UARTE0), edge (GPIOTE) | LED P0.13; Serial1 TX/RX P1.03/P1.10 to confirm |
| Arduino Nano 33 IoT | SAMD21G18A / samd21 | `bossac` over the serial port (`NEURON_BOSSAC`) | GPIO, tick | UART and edge interrupt LEFT OUT (SERCOM/EIC need the board); LED PA17 to confirm; runs at 8 MHz |
| Arduino UNO Q (MCU side) | STM32U585 / stm32u5 | st-flash / probe-rs / OpenOCD (SWD) | GPIO, tick, UART (USART1), edge (EXTI) | LED PA5 placeholder, USART1 TX PA9 to confirm; the Linux side is a separate `host` node |

First build steps for a Tier 3 board (install `arm-none-eabi-gcc` first, env `NEURON_ARM_GCC`):
1. `neuron check examples/corpus/mcu/m01_blink.neuron` with the module targeting the board,
   or `neuron emit-c <f> --board <board> -o out` to see the generated port.
2. `neuron build <f> --board <board>` — this is the first real arm-none-eabi-gcc compile;
   fix any compile error in the port C (the register data is from the public manuals).
3. Flash with the tool in the table; confirm the LED pin and UART pin against the board.

## Host node (not flashed)

| Device | Command | Session |
|---|---|---|
| Raspberry Pi 5 | `neuron emit python <host module>` → run on the Pi; `raspberry_pi_5` manifest | I4 |

## FPGA (if obtained)

| Board | Command | Tool |
|---|---|---|
| Artix-7 (matches `artix7_35t_board`) | `neuron emit-verilog <f> --board artix7_35t_board` then build the bitstream | openFPGALoader or Vivado |
| Spartan | NOT in the model yet — needs a `.fpga.toml`; tell Kiro the exact part (e.g. XC7S25) to add it | — |

## Measurements to take (session I1 — these replace the generic numbers in Phases D and E)

- PLC: input filter time, scan cycle.
- Robot: command latency.
- Valves: switching time.
- Link media: real transport and jitter per medium.
Write them into a timing profile TOML and run `neuron cell check --profile <file>` (E6.3);
then the manifests can move from `trust = "generic"` to `measured`.
