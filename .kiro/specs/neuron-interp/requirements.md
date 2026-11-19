# Requirements: the reference interpreter (task A10)

Status: v1.0 (2026-11-16). Parent spec: `neuron-language`. Normative text:
`spec/language/17-interpreter.md`; the models of computation are in chapter 14.

## Context

The analysis (A4 to A8) and NIR (A9) say whether a program is well formed. Nothing yet
says what it *does*. Every later component (the instruction-set simulators, the code
generators, the emitters, the importers and the digital twin) is checked against an
executable meaning of the language, so that meaning has to exist first, and it has to be
usable without hardware.

## Requirements

| Id | Requirement | Acceptance |
|---|---|---|
| I1 | Run a NIR package in simulated time, with no wall clock and no hidden randomness | the same package, options and scenario give the same trace (MOC-R1) |
| I2 | Tasks, interrupts, rules, state machines, sequences, PLC scans and the safety logic follow chapters 6, 8 and 14 | one scenario test per activity kind |
| I3 | Signals have logical execution time | a task reads signals as of its release and writes become visible at its deadline; property LET-1 holds under random execution delays |
| I4 | Faults enter the safe state and stop all but the safety logic | property SAFE-1 on generated scenarios; reset resumes the program |
| I5 | Numbers follow chapter 3: exact integers with overflow traps, quantities in base units with their display unit | tests for traps, units and affine temperatures |
| I6 | Standard function blocks the examples need are available | `ton`, `tof`, `tp`, counters, edge detectors, `sr`/`rs`, `pid`, filters, `ramp`, `hysteresis`, `debounce` |
| I7 | Devices outside the package are modeled, not ignored | `run` starts an action with BUSY / DONE and timeouts take the declared branch |
| I8 | Scripted inputs | scenario files (`at 100 ms: button = true`) and the same operations inside `test` blocks |
| I11 | A `test` block runs its own module and the modules it refers to; a test in a `cell` runs the whole package (chapter 17.8) | a fault in an unrelated module of the same package does not change the result of a test; `neuron test examples` gives the same results as testing each folder |
| I9 | `neuron run` and `neuron test` | `run` prints the trace; `test` prints PASS / FAIL with the location of a failed assertion and exits 1 on failure |
| I10 | Total: no panics and no hangs on any valid program | every example, every valid spec program and generated variations run |

## Out of scope

`hdl` circuits (B7), robot trajectories and workspace checks (C3, D4), link latency and
clock drift (D3), class invariants, peripheral registers (B1, B6).
