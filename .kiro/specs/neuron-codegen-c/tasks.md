# Tasks: MIR, C backend and runtime (B2, B3)

- [ ] 1. `neuron-mir`: data model, lowering for tasks, gpio, integers, `if`; verifier; printer.
- [ ] 2. `runtime/neuron-rt` core and host port; scheduler with SRP; safe state.
- [ ] 3. `neuron-codegen-c`: emitter for the subset of task 1; `neuron emit-c`.
- [ ] 4. Differential test MOC-C1 against the interpreter on the blink example.
- [ ] 5. `uart` print, interrupts, `safety` and watchdog; `NE0610` for unsupported constructs.
- [ ] 6. AVR port and `neuron build` for the UNO (needs the targets of B1 task 2).
- [ ] 7. `fsm`, `match`, bounded loops; source maps.
- [ ] 8. Golden files, error docs, evidence entry in `docs/kiro-log.md`.
