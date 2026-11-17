# Tasks: MIR, C backend and runtime (B2, B3, B4)

- [x] 1. `neuron-mir`: data model, lowering for tasks, gpio, integers, `if`; verifier; printer.
- [x] 2. `runtime/neuron-rt` core and host port; scheduler; safe state.
      Tasks run to completion in priority order inside the 1 ms tick, so tasks never
      preempt each other; interrupts do, and data they share with tasks is `volatile`
      and updated inside atomic sections (the stack-resource ceiling of that data is the
      interrupt level). Preemptive task scheduling with SRP ceilings is left for B8.
- [x] 3. `neuron-codegen-c`: emitter for the subset of task 1; `neuron emit-c`.
- [x] 4. Differential test MOC-C1 against the interpreter on the blink example (and on
      interrupts, `fsm`, signals, events, faults and random inputs).
- [x] 5. `uart` print, interrupts, `safety` and watchdog; `NE0610` for unsupported constructs.
- [x] 6. AVR port and `neuron build` for the UNO (and the Mega); `neuron flash` through
      `avrdude`. The first flash of a real board is gate G2 (needs the hardware).
- [~] 7. `fsm`, bounded loops and source maps are done. `match`, enums, arrays, classes,
      `sequence` and the native function blocks report `NE0610` (listed in
      `docs/targets.md`).
- [x] 8. Golden files (`conformance/golden/c/`), error docs (`docs/errors/targets.md`),
      evidence entry in `docs/kiro-log.md`.
