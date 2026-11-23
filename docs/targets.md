# Targets and code generators (Phase B, tasks B1 to B9)

This page says what runs where today. It is the reference for the `NE0610` and "described
board" messages of the tool. The FPGA flow is in `docs/hdl.md`, the native AVR backend in
`docs/native-backend.md`, the displays in `docs/ui.md`.

## How a program reaches a board

```
.neuron ─ analysis ─▶ NIR ─ hardware checks ─▶ bindings ─ lowering ─▶ MIR ─┬─▶ C ─ gcc ─▶ image ─ flash tool ─▶ board
          (A4–A8)          (neuron-target, B1)            (neuron-mir, B2)  │   (neuron-codegen-c, B2–B5; neuron-ui, B9)
                                                                            └─▶ AVR machine code (neuron-native-avr, B8)
hdl modules ─▶ Verilog + constraints (neuron-codegen-hdl, B7) ─ yosys/nextpnr or Vivado ─▶ bitstream ─ openFPGALoader ─▶ FPGA
```

| Command | What it does |
|---|---|
| `neuron targets [board]` | Lists the boards (MCU and FPGA) with their tier, or prints one board. |
| `neuron check prog.neuron` | Semantic analysis plus the hardware checks against the board after `on` (`NE0601`–`NE0608`). |
| `neuron emit-c prog.neuron [-o dir]` | Prints the generated C, or writes it with the runtime of the board's port. |
| `neuron build prog.neuron [--board b] [--out dir]` | Builds the firmware of the board (table below). |
| `neuron build prog.neuron --host` | A host executable of the same C that runs in simulated time. |
| `neuron build prog.neuron --native` | UNO firmware from the native backend, with no external tool. |
| `neuron emit-verilog prog.neuron [--board fpga] [-o dir]` | Verilog, and the PCF/XDC of an FPGA board. |
| `neuron flash prog.neuron [--port P \| --drive D]` | Builds and writes the board with its programmer. |

`--board` replaces the board named in the source, so one program can be checked against
several boards.

## Boards

Every board file has `trust = "community"`: the facts come from public documentation and
have not been checked on the device by the maintainers yet.

| Board | Chip | Tier | Build output | Flash | Checked without hardware |
|---|---|---|---|---|---|
| `arduino_uno_r3` | ATmega328P, 16 MHz | runtime | `app.hex` (avr-gcc or `--native`) | avrdude | C build, AVR simulator, native backend |
| `arduino_mega_2560` | ATmega2560, 16 MHz | runtime | `app.hex` | avrdude | C build |
| `raspberry_pi_pico` | RP2040, 12 MHz crystal | runtime | `app.elf`, `app.bin`, `app.uf2` (RAM image) | UF2 drive | C build, Thumb simulator against the interpreter |
| `raspberry_pi_pico_2w` | RP2350, 12 MHz crystal | runtime | `app.elf`, `app.bin`, `app.uf2` | UF2 drive | C build (not simulated) |
| `nucleo_f446re` | STM32F446, 16 MHz HSI | runtime | `app.elf`, `app.bin` (flash image) | probe-rs, OpenOCD or st-flash | C build (not simulated) |
| `nano_33_ble_sense` | nRF52840, 64 MHz HFINT | runtime | `app.elf`, `app.bin`, `app.uf2` (flash image above the UF2 bootloader) | UF2 drive (NANO33BLE) | model loads; C not compiled here (no arm-gcc). GPIO + tick + UARTE0 TX + GPIOTE edge interrupts; ADC/PWM left out. Serial1 pins to confirm on the board |
| `nano_33_iot` | SAMD21G18A, 8 MHz OSC8M | runtime | `app.elf`, `app.bin` (flash image above the SAM-BA bootloader) | bossac | model loads; C not compiled here. GPIO + tick only; UART TX and EIC edge interrupts left out (SERCOM/EIC need GCLK, deferred); ADC/PWM left out. LED (PA17) and TX (PA22) pins to confirm |
| `uno_q` | STM32U585 (MCU side), 16 MHz HSI | runtime | `app.elf`, `app.bin` (flash image) | probe-rs, OpenOCD or st-flash (SWD) | model loads; C not compiled here. GPIO + tick + USART1 TX + EXTI edge interrupts; ADC/PWM left out. Linux side out of scope. User LED pin is a placeholder (PA5), to confirm on the board |
| `ek_tm4c123gxl` | TM4C123GH6PM (Tiva C), 16 MHz PIOSC | runtime | `app.elf`, `app.bin` (flash image at 0x00000000) | lm4flash (on-board ICDI) | C **compiled clean** with arm-none-eabi-gcc (app.bin 2168 B, stack top 0x20008000, reset vector in flash with the Thumb bit). GPIO + tick only; UART0 TX and per-port edge interrupts left out on purpose, ADC/PWM left out. LED is PF1 (red). Not run on hardware |
| `esp32_s3_devkit` | ESP32-S3 | sdk | an `idf.py` project, built when ESP-IDF is installed | esptool | project generation |
| `xiao_esp32s3` | ESP32-S3 | sdk | an `idf.py` project | esptool | project generation |
| `nucleo_n657x0_q` | STM32N657 | described | — | — | pins and clocks only (placeholder) |

FPGA boards: `artix7_35t_board` (XDC) and `ice40up5k_breakout` (PCF); see `docs/hdl.md`.

**Tiers.** *runtime*: the tool builds the image and knows how to flash it. *sdk*: the port is
written against the vendor SDK; the tool writes the SDK project and builds it when the SDK
is installed. *described*: pins, clocks and rates are checked and the C is generated, but
there is no runtime port, so `neuron build` stops with a clear message.

**What is not verified yet.** No image has run on a real board: that is the hardware gate
G2 (UNO) and session I2 (the others). The Pico image is run by `neuron-sim-thumb`, which
models the RP2040 behavior the port depends on; the RP2350, STM32 and ESP32-S3 images are
built but not executed. The RAM UF2 start (the boot ROM taking SP and PC from the vector
table at `0x20000000`) and the RP2350 image definition block need confirmation on the boards.

The three Arduino ports added for the nRF52840 (`nano_33_ble_sense`), SAMD21G18A
(`nano_33_iot`) and STM32U585 (`uno_q`) were written against the public reference manuals and
checked by the target model and the code generator, but the C was **not compiled** on the
machine that added them (no `arm-none-eabi-gcc` there) and no image has run on the boards. On
the SAMD21 the UART TX and the EIC edge interrupts are left out on purpose (SERCOM and the EIC
both need GCLK routing that is risky to get right without the board); a program that only uses
GPIO and the tick runs, one that needs UART output or an edge interrupt builds but those do
nothing. The `uno_q` on-board user LED pin is a placeholder (PA5); the real pin is to confirm
on the board.

The TM4C123GH6PM port (`ek_tm4c123gxl`) was written against the public datasheet (SPMS376)
and its C **was** compiled with `arm-none-eabi-gcc` on the machine that added it (unlike the
three Arduino ports above): `app.bin` is 2168 bytes, the vector table carries the stack top
0x20008000 and the reset vector in flash with the Thumb bit. It has not run on hardware yet.
The TM4C GPIO is not the STM32 layout (per-port clock enable through SYSCTL RCGCGPIO, a
non-linear per-port base table, address-masked data writes), so only GPIO in/out, input
pull-up and the 1 ms SysTick tick are implemented. UART0 TX and the per-port edge interrupts
are left out on purpose until the board session (the GPIO interrupt is per-port, not per-line
like STM32 EXTI, and is risky to wire blind); a program that only uses GPIO and the tick runs,
one that needs UART output or an edge interrupt builds but those do nothing.

## Runtime ports (`runtime/neuron-rt/port/`)

| Port | GPIO | Edge interrupts | Tick | UART TX | PWM / ADC | Display bus |
|---|---|---|---|---|---|---|
| `avr` | ✔ | INT0–INT7 | timer 0 CTC | UART0, ring buffer | timers 1/2, ADC | TWI, SPI |
| `rp` (RP2040, RP2350) | SIO | IO_BANK0 | SysTick | UART0 on GP0 | not yet (`NE0610`) | not yet |
| `stm32f4` | ✔ | EXTI | SysTick | USART2 on PA2 | not yet (`NE0610`) | not yet |
| `stm32u5` (STM32U585) | ✔ | EXTI | SysTick | USART1 on PA9 | not yet (`NE0610`) | not yet |
| `nrf52` (nRF52840) | ✔ (P0, P1) | GPIOTE | SysTick | UARTE0 (EasyDMA, 1 byte) | not yet (`NE0610`) | not yet |
| `samd21` (SAMD21G18A) | ✔ (PA, PB) | not yet (deferred) | SysTick | not yet (deferred) | not yet (`NE0610`) | not yet |
| `tm4c` (TM4C123GH6PM) | ✔ (A–F, RCGCGPIO + base table, masked writes) | not yet (deferred) | SysTick | not yet (deferred) | not yet (`NE0610`) | not yet |
| `esp_idf` | gpio driver | GPIO ISR service | esp_timer | uart driver | not yet | I2C, SPI drivers |
| `host` | simulated | from the scenario | simulated time | traced | traced | traced (`BUS` lines) |

The core (`nrt_core.c`) follows the scheduling rules of the interpreter
(`crates/neuron-interp/src/sched.rs`): in each millisecond, signal commits first, then inputs
and interrupts, the safety poll, task releases, and the ready tasks by priority. A release
that falls in a fault is dropped; a fault aborts the activity.

## The subset of the C backend

The interpreter (`neuron run`) runs every valid program. The C backend translates this
subset and reports anything else as `NE0610`, naming the construct; it never drops code.

Supported: `task ... every` and `task ... on <event>`, priorities, `deadline`, `signal` with
its logical execution time; `on interrupt <pin>.rising|falling|change`; `fsm` with `when` and
`after`, `when` rules; `safety:` with `safe_state`, `watchdog`, rules, `reset when` and
`on fault`; `gpio`, `pwm`, `adc`, `uart` (`print` with `{name}`); integers with overflow
checks, `f32` quantities in SI base units, `bool`; `if`, `for` over a constant range, bounded
`while`; user functions with scalar parameters, `scale`, `abs`, `min`, `max`, `clamp`; one
`display` with its first `screen` (B9).

Not yet (`NE0610`): `match`, classes, enums, arrays, `sequence`, the native function blocks
(`ton`, `tof`, `pid`, `debounce`, `hysteresis`, ...).

## How it is tested

| Test file | What it shows |
|---|---|
| `crates/neuron-target/tests/b1_targets.rs` | Target files load (TGT-1), exclusive ownership (HW-1), clock solutions (CLK-1), each hardware code, the tier of every board. |
| `crates/neuron-codegen-c/tests/b2_codegen.rs` | Host builds of the C give the interpreter's trace; goldens; UNO and Mega builds. |
| `crates/neuron-codegen-c/tests/b5_ports.rs` | RP2040 firmware in the Thumb simulator matches the interpreter (blink, interrupt, fsm, signal, UART); STM32 and RP2350 images build; UF2 format; ESP-IDF project. |
| `crates/neuron-sim-avr/tests/b6_avr.rs` | avr-gcc firmware in the ATmega328P simulator matches the interpreter. |
| `crates/neuron-sim-thumb/tests/b6_thumb.rs` | ARMv6-M core, SysTick, NVIC, GPIO with hand-assembled programs. |
| `crates/neuron-codegen-hdl/tests/b7_hdl.rs` | Verilog goldens, constraints, `NE0702`, iverilog simulation, yosys synthesis. |
| `crates/neuron-native-avr/tests/b8_native.rs` | Native images in the AVR simulator match the interpreter. |
| `crates/neuron-ui/tests/b9_ui.rs` | Firmware display traffic rebuilds the reference image pixel for pixel; strategies and `NE0604`. |
| `crates/neuron-cli/tests/b4_cli.rs` | The commands, run as a user runs them. |

CI installs avr-gcc, arm-none-eabi-gcc, iverilog and yosys, and sets `NEURON_REQUIRE_*` so
none of these tests can skip there.

## Toolchains

`tools/install-toolchains.ps1` installs gcc (w64devkit), avr-gcc, avrdude and
arm-none-eabi-gcc under `%USERPROFILE%\toolchains`, verifying every download with SHA-256,
and `. .\tools\toolchain-env.ps1` puts them on `PATH` for the session. ESP-IDF, OpenOCD,
probe-rs, yosys, nextpnr and openFPGALoader are optional and found on `PATH`. See
`docs/phase-b-prerequisites.md`.

This is a design aid. The generated firmware is not certified for safety functions, and a
hardwired emergency stop stays the authority.
