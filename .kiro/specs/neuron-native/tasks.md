# Tasks: native AVR backend (B8)

- [x] 1. Assembler, encodings and Intel HEX, with unit tests.
- [x] 2. Code generator: layout, start-up, tick, scheduler, faults, activities, interrupts.
- [x] 3. Expressions and statements of the subset; `NE0610` outside it.
- [x] 4. Tests in `neuron-sim-avr` against the interpreter (blink, button, fsm, signal, interrupts, overflow) and two property tests.
- [x] 5. CLI: `neuron build --native`, `neuron flash --native`.
- [x] 6. Docs (`docs/native-backend.md`), evidence in `docs/kiro-log.md`.
- [ ] 7. Flash a native image on a real UNO (with gate G2, needs the board).
- [ ] 8. RV32IMC (Pico 2 W) and Thumb-2 native backends; floats and UART in the native subset.
