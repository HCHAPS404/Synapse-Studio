# Requirements: target model (task B1)

Source: `spec/language/07-hardware.md`, `docs/roadmap.md` task B1.

A *target* is the description of the machine a program runs on. Adding a device adds
files, never compiler code (spec 07, first page).

## User stories

- As a student, I want to write `module blink for mcu on arduino_uno_r3` and have the
  tool know that `D13` is an output pin, so that I do not read a datasheet to blink an LED.
- As a control engineer, I want a wrong pin, a clock I cannot reach or a pin used twice
  to be reported before I flash anything, with the nearest valid choice.
- As a maintainer, I want to add a board by adding files, so that the compiler does not
  change when the lab buys a new one.

## Acceptance criteria (EARS)

1. THE repository SHALL contain `targets/arch/`, `targets/chips/`, `targets/boards/`
   and `targets/runtimes/` with one description per supported device (UNO R3, Mega 2560,
   Raspberry Pi Pico, Pico 2 W, ESP32-S3, XIAO ESP32-S3, NUCLEO-N657X0-Q).
2. WHEN `neuron check` reads `module m for mcu on <board>` THE SYSTEM SHALL load the
   board, its chip, its architecture and its runtime, and SHALL report `NE0607` when a
   name is unknown or a file is invalid.
3. WHEN an `io` binding names a pin that does not support the requested function THE
   SYSTEM SHALL report `NE0603` and list the pins that do.
4. WHEN two `io` bindings or `uses regs` declarations own the same resource THE SYSTEM
   SHALL report `NE0601` (property HW-1).
5. WHEN a `clocks:` block asks for a frequency the clock tree cannot make THE SYSTEM
   SHALL report `NE0605` with the nearest achievable values.
6. WHEN a derived rate (baud, PWM, timer) misses its tolerance THE SYSTEM SHALL report
   `NE0606` with the achieved error.
7. WHEN a board file is loaded twice THE SYSTEM SHALL give the same description
   (deterministic, no dependence on file order).
8. THE target files SHALL be derived with attribution and SHALL NOT copy vendor manuals,
   datasheets or register tables verbatim (repository convention).
9. THE `NE0601`, `NE0603`, `NE0605`, `NE0606` and `NE0607` codes SHALL each have an
   entry in `docs/errors/` and in `spec/language/10-diagnostics.md`.

## Properties

- **HW-1** (exclusive ownership), checked by a property test on random sets of bindings.
- **CLK-1**: a solution returned by the clock solver always respects the chip's limits,
  and the reported error equals `|achieved - requested| / requested`.
- **TGT-1**: every board file in `targets/` loads and validates (a test over the folder).

## Out of scope for this task

SVD import (B1b, after G2), the DMA solver, multicore assignment, `.target` files for
non-MCU devices (D1).
