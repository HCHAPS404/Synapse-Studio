# Phase B prerequisites

What has to exist before the first program runs on a real board (gate G2, 2026-11-21).
Items marked **You** cannot be done by the agent.

## Already in place (end of Phase A)

- Language, analyzer, NIR, interpreter, formatter, language server and standard library,
  tagged `spec-v1.0-rc1`.
- Specs for the first two Phase B tasks: `.kiro/specs/neuron-targets/` (B1) and
  `.kiro/specs/neuron-codegen-c/` (B2, B3).
- A reference trace for every example (`neuron run`), which the C backend must match.

## Tools to install (You)

| Tool | Why | How to check |
|---|---|---|
| A C compiler for the host (MSVC or GCC via MSYS2/MinGW) | differential tests of generated C | `cc --version` or `gcc --version` |
| `avr-gcc` and `avr-libc` | build firmware for UNO and Mega | `avr-gcc --version` |
| `avrdude` | flash the UNO and Mega | `avrdude -?` |
| Arduino UNO R3 and a USB cable | gate G2 | the board appears as a COM port in Device Manager |
| `arm-none-eabi-gcc` (xPack 15.2.1, installed by `tools/install-toolchains.ps1`) | RP2040, RP2350 and STM32F4 firmware | `arm-none-eabi-gcc --version` |
| ESP-IDF 5.x (optional) | build the generated ESP32-S3 project | `idf.py --version` |
| probe-rs, OpenOCD or st-flash (optional) | flash the NUCLEO-F446RE over SWD | `probe-rs --version` |
| esptool (optional, ships with ESP-IDF) | flash the ESP32-S3 | `esptool.py version` |
| Icarus Verilog and Yosys (optional locally, installed in CI) | simulate and synthesize the emitted Verilog | `iverilog -V`, `yosys -V` |
| nextpnr-ice40 / icepack, or Vivado, and openFPGALoader (optional) | bitstream and FPGA flashing (session I3) | `openFPGALoader --help` |

Quick way on Windows: install the Arduino IDE or `arduino-cli`; it ships `avr-gcc` and
`avrdude`. Give the agent the folder of those tools if they are not on `PATH`.

## Status at the end of Phase B (2026-11-17)

Everything in B1–B9 that can run without a board is done and tested. What remains
needs the devices (**You**):

| Session | What to run | Command |
|---|---|---|
| G2 | blink, UART and interrupt on the UNO, C and native builds | `neuron flash examples/corpus/mcu/m01_blink.neuron --port COMx` (add `--native` for B8) |
| I2 | the same program on Mega, Pico, Pico 2 W, NUCLEO-F446RE, ESP32-S3, XIAO | `neuron flash <file> --board <board> [--port COMx \| --drive E:]` |
| I3 | counter, PWM and UART TX on the lab FPGA | `neuron flash examples/corpus/hdl/d01_counter.neuron --board <fpga>` |
| B9 | the dashboard on an ESP32-S3 with a TFT | install ESP-IDF, then `neuron flash <ui package> --board esp32_s3_devkit --port COMx` |

## Decisions taken

- C backend first, native machine code later (B8).
- UNO R3 first for G2; other boards follow in B5.
- The runtime is C with no heap and no RTOS.

## Open decisions (You)

- Which board is the first real target if the UNO is not available (Mega 2560 or Pico).
- Whether the lab computer has the toolchain, or flashing is done from another machine.

## Known gaps carried over from Phase A

- The parser reads `u8 <` as the start of generic arguments (`docs/usability-review.md`, item 4).
- `drive()` of a PLC input needs a quoted name (item 6).
- Line breaking at 88 columns in the formatter and the independent review of the
  specification are still open.
