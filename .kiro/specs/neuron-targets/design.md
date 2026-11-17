# Design: target model (task B1)

## Crate

`crates/neuron-target`: loader, validation and solvers. It depends on `serde`, `toml` and
`thiserror`. `neuron-sema` calls it when a module names a board (a new pass after name
resolution, before the time analysis, which can then use the real clock).

## Files

```text
targets/arch/avr8.arch.toml            ISA, word size 8, Harvard, no FPU, AVR vector interrupts
targets/arch/thumbv6m.arch.toml        Cortex-M0+ (RP2040)
targets/arch/thumbv8m.arch.toml        Cortex-M33 (RP2350, N657)
targets/arch/xtensa-lx7.arch.toml      ESP32-S3
targets/chips/atmega328p.chip.toml     memory map, peripherals, pins and alternate functions
targets/boards/arduino_uno_r3.board.toml   pin names (D13, A0), LED, crystal, programmer
targets/runtimes/baremetal-avr.toml    startup, vector table, scheduler port
```

Fields common to every file: `schema = 1`, `name`, `source` (where the facts come from and
the licence), `trust` (`verified`, `community`, `ai-generated`).

## Model

```text
Board  -> chip name, crystal Hz, named pins {D13 -> PB5}, on-board parts, programmer
Chip   -> arch name, memory regions, peripherals {name, kind, instances}, pins
          {PB5: [gpio, spi.sck, ...]}, clock tree (sources, PLL limits, dividers)
Arch   -> word bits, endianness, Harvard, FPU, atomic width, interrupt model, ABI name
Runtime-> startup symbol, scheduler port, tick source
```

Everything is a plain Rust struct with `BTreeMap` (deterministic order).

## Solvers

- **Pin solver.** For each `io`, find the chip pin(s) for the board name, check the function
  is in the pin's list, and take ownership. A second owner is `NE0601`.
- **Clock solver.** Search the PLL multipliers and dividers of the chip file for the
  requested frequency. Exact solutions first, then the nearest on each side (`NE0605`).
  Derived rates use the same arithmetic with integer divisors (`NE0606` above tolerance).
- Both are pure functions of the target and the program, so results are reproducible.

## First vertical slice (gate G2)

Only the Arduino UNO R3 is complete first: ATmega328P with `gpio`, `uart0`, `timer0/1/2`
and external interrupts INT0/INT1. The other boards get their files as B5 reaches them;
the loader and the solvers are written once.

## Data sources and attribution

Facts such as pin numbers and peripheral names are not copyrightable, but the register
tables and prose of datasheets are. Files carry a `source` field and are written in our own
words and layout. SVD import is a later task and records the SVD licence.
