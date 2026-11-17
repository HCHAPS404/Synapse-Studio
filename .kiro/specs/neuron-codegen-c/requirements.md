# Requirements: MIR, C backend and runtime (tasks B2, B3)

Source: `docs/roadmap.md` tasks B2 and B3, gate G2 (2026-11-21).

## User stories

- As a student, I want `neuron build blink.neuron` to produce firmware for my board
  without opening a vendor IDE.
- As a control engineer, I want the generated C to keep its source locations, so that a
  fault on the device points at my Neuron line.
- As a judge, I want the same program to run in the interpreter and on the simulated
  device with the same trace.

## Acceptance criteria (EARS)

1. WHEN `neuron emit-c <package>` runs on a package that analyzes without error THE SYSTEM
   SHALL write portable C99 and a source map.
2. THE generated C SHALL compile with `-Wall -Wextra -Werror` on the host compiler.
3. WHEN the generated code runs on the host runtime with the same scenario THE SYSTEM
   SHALL produce the same `OUT` trace as `neuron run` for every example of `examples/`
   that is in the supported subset (property **MOC-C1**).
4. THE runtime `neuron-rt` SHALL provide startup, the static SRP scheduler, timers,
   interrupt dispatch, the safe state and the watchdog, with no heap allocation.
5. WHEN a program uses a construct outside the supported subset THE SYSTEM SHALL report
   `NE0610` naming the construct and the task that adds it (no silent omission).
6. WHEN a fault or trap occurs THE generated code SHALL apply the `safe_state:` writes
   before anything else (property SAFE-1 on the host runtime).
7. WHEN the target is the Arduino UNO R3 THE SYSTEM SHALL produce an image that blinks,
   prints over the serial port and handles an interrupt (gate G2, with the hardware).

## Supported subset for G2

Tasks (`every`, `on interrupt`), `io` of `gpio`, `uart` (print) and `timer`, `var` and
`signal` of integer, bool and fixed-width types, `if`, `match`, bounded `for`, functions,
`safety:` with `safe_state` and a watchdog, `fsm`. Floats, classes, `sequence` and
`plc` constructs follow later (B2 second pass).

## Out of scope

Native machine code (B8), LLVM, vendor SDK ports (B5), the `plc`, `hdl`, `robot` and `ui`
dialects (their own emitters).
