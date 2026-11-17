# Design: MIR, C backend and runtime (tasks B2, B3)

## Pipeline

```text
NIR (A9) --lower--> MIR (SSA, typed, units erased to base integers/fixed point)
MIR --C emitter--> out/<pkg>.c + out/<pkg>.map.json
out/<pkg>.c + runtime/neuron-rt/*.c --cc--> firmware
```

## Crates and folders

- `crates/neuron-mir`: MIR data structures, lowering from NIR, a verifier (types, SSA
  dominance) and a printer for golden files.
- `crates/neuron-codegen-c`: emitter and source map.
- `runtime/neuron-rt/`: the C runtime. `core/` (portable scheduler, SRP, safe state),
  `port/host/` (timers from the OS, for tests) and `port/avr/` (timer0 tick, vector table).

## Lowering rules (what changes from NIR)

- Units disappear after checking: quantities become integers in a fixed base (`ns`, `um`,
  `mdeg`), chosen per dimension, with the conversion written in the MIR.
- `signal` becomes two buffers and a commit at the deadline (LET), matching chapter 17.
- Tasks become functions with a static release table; the scheduler is generated, not
  dynamic. Priorities and ceilings come from the analysis (A7).
- `fsm` becomes a `switch` on a state variable; `after D` uses a per-machine entry time.
- Every trap point (overflow, division) calls `nrt_fault(code)`, which applies the safe
  state first.

## Source maps

Each emitted C statement has a `#line` directive pointing to the Neuron file, plus a JSON
map (`c line -> neuron span`) for the debugger and the Studio.

## Testing

- Golden MIR and C for the examples (`conformance/golden/mir`, `conformance/golden/c`).
- Differential test **MOC-C1**: build with the host `cc`, run with a scenario file, compare
  the `OUT` lines with the interpreter. The test skips with a clear message when no C
  compiler is installed, and CI installs one.
- Property tests: MIR verifier accepts every lowered example; random scenarios keep SAFE-1.

## Order of work

1. MIR for tasks, `io gpio`, integers and `if` (blink). 2. Host runtime and the first
differential test. 3. `uart`, interrupts, `safety`. 4. AVR port and image for the UNO.
5. `fsm`, `match`, bounded loops. 6. Source maps.

## Decision: C first, native later

The first path to the UNO is generated C compiled by `avr-gcc` and flashed by `avrdude`
(a toolchain the user installs, see `docs/phase-b-prerequisites.md`). Native AVR code
without an external compiler is B8 and reuses MIR unchanged.
