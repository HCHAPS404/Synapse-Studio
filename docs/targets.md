# Targets and the C backend (Phase B, tasks B1 to B6)

This page says what runs where today. It is the reference for the `NE0610` and
"described board" messages of the tool.

## How a program reaches a board

```
.neuron ─ analysis ─▶ NIR ─ hardware checks ─▶ bindings ─ lowering ─▶ MIR ─▶ C ─ avr-gcc ─▶ app.hex ─ avrdude ─▶ board
          (A4–A8)          (neuron-target, B1)            (neuron-mir, B2)  (neuron-codegen-c, B2–B4)
```

| Command | What it does |
|---|---|
| `neuron targets [board]` | Lists the boards with their tier, or prints one board (pins and their functions, clocks, runtime, programmer). |
| `neuron check prog.neuron` | Semantic analysis plus the hardware checks against the board after `on` (`NE0601`–`NE0608`). |
| `neuron emit-c prog.neuron [-o dir]` | Prints the generated C, or writes it with the runtime sources and the source map (`app.map.json`). |
| `neuron build prog.neuron [--board b] [--out dir]` | Builds `build/<board>/app.hex` and `app.elf` and prints the memory use. |
| `neuron build prog.neuron --host` | Builds a host executable of the same C that runs in simulated time and prints the trace format of `neuron run`. |
| `neuron flash prog.neuron --port COM3` | Builds, then writes the board with `avrdude` (protocol and speed from the board file). |

`--board` replaces the board named in the source, so one program can be checked
against several boards.

## Boards

Every board file has `trust = "community"`: the facts come from public documentation
and have not been checked on the device by the maintainers yet.

| Board | Chip | Tier | What works |
|---|---|---|---|
| `arduino_uno_r3` | ATmega328P, 16 MHz | runtime | check, emit-c, build, flash, AVR simulator |
| `arduino_mega_2560` | ATmega2560, 16 MHz | runtime | check, emit-c, build, flash |
| `raspberry_pi_pico` | RP2040 | described | check, emit-c, host build |
| `raspberry_pi_pico_2w` | RP2350 | described | check, emit-c, host build |
| `esp32_s3_devkit` | ESP32-S3 | described | check, emit-c, host build |
| `xiao_esp32s3` | ESP32-S3 | described | check, emit-c, host build |
| `nucleo_f446re` | STM32F446 | described | check, emit-c, host build |
| `nucleo_n657x0_q` | STM32N657 | described | check only; placeholder (pin list incomplete, external flash, crystal to be confirmed) |

*Described* means the pins, the clock tree and the derived rates are checked, and the
C is generated, but the runtime has no port for the chip yet, so `neuron build` stops
with a clear message. The ports for RP2040 and ESP32-S3 are the next step of B5.

`neuron flash` has been run against the tool chain only; the first flash of a real UNO
is gate G2 and needs the board on a USB port.

## The subset of the C backend

The interpreter (`neuron run`) runs every valid program. The C backend translates this
subset and reports anything else as `NE0610`, naming the construct; it never drops code.

Supported:

- `task ... every` and `task ... on <event>`, priorities, `deadline`, `signal` with its
  logical execution time;
- `on interrupt <pin>.rising|falling|change` (external interrupts), with data shared
  between interrupts and tasks made `volatile` and updated atomically;
- `fsm` with `when` and `after` transitions, `when` rules;
- `safety:` with `safe_state`, `watchdog`, rules, `reset when` and `on fault`;
- `gpio.input`, `gpio.output` (`write`, `toggle`, `read`, edges), `pwm` (`duty`),
  `adc` (`read`), `uart` (`print` with `{name}` interpolation);
- integers with overflow checks (a fault, like in the interpreter), `f32` quantities
  in SI base units, `bool`; `if`, `for` over a constant range, bounded `while`;
- user functions with scalar parameters, `scale`, `abs`, `min`, `max`, `clamp`.

Not yet (`NE0610`): `match`, classes, enums, arrays, `sequence`, and the native function
blocks of the standard library (`ton`, `tof`, `pid`, `debounce`, `hysteresis`, ...).

Integer arithmetic is computed in 64 bits and checked against the declared type, so an
overflow faults on the chip at the same point as in the interpreter. Units are erased:
a quantity is an `f32` in SI base units.

## Runtime

`runtime/neuron-rt` is C99 and builds with `-Wall -Wextra -Werror`.

- `nrt_core.c` follows the scheduling rules of the interpreter
  (`crates/neuron-interp/src/sched.rs`): in each millisecond, signal commits first, then
  inputs and interrupts, the safety poll, task releases, and the ready tasks by
  priority. A release that falls in a fault is dropped; a fault aborts the activity.
- The AVR port uses `timer0` in CTC mode for the 1 ms tick (so `timer0` pins are not
  available for PWM, `NE0601`), `INT0`–`INT7`, a UART transmit ring buffer, PWM on
  `timer1` and `timer2`, the ADC, the watchdog and idle sleep.
- The host port runs in simulated time: `app_host --for <ms> [scenario-file]`.

## How it is tested

- `crates/neuron-target/tests/b1_targets.rs`: every target file loads (TGT-1), exclusive
  ownership (HW-1, property test), clock solutions respect the limits (CLK-1, property
  test), each hardware code.
- `crates/neuron-codegen-c/tests/b2_codegen.rs`: the host build of the generated C gives
  the same trace as the interpreter (blink, interrupts, fsm, signals, events, faults and
  reset, random button presses as a property test); golden C files in
  `conformance/golden/c/`; UNO and Mega builds without warnings.
- `crates/neuron-sim-avr/tests/b6_avr.rs`: the firmware that avr-gcc builds runs in the
  ATmega328P simulator, and its pins change at the same milliseconds as the
  interpreter's outputs (blink, button interrupt, traffic light, two tasks with a
  signal, interrupt priorities, ADC with float math and the watchdog, UART text, PWM
  compare value, random presses as a property test).
- `crates/neuron-cli/tests/b4_cli.rs`: the commands, run as a user runs them.

## Toolchains

`tools/install-toolchains.ps1` installs gcc (w64devkit), avr-gcc and avrdude under
`%USERPROFILE%\toolchains`, verifying every download with SHA-256, and
`. .\tools\toolchain-env.ps1` puts them on `PATH` for the session. On Linux CI the
packages `gcc-avr` and `avr-libc` are used. See `docs/phase-b-prerequisites.md`.

This is a design aid. The generated firmware is not certified for safety functions, and
a hardwired emergency stop stays the authority.
