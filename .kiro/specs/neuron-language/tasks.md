# Tasks: Neuron language spec (plan Task 5)

- [x] 1. Write the language specification chapters 01–10 in `spec/language/`
  - _Requirements: R1–R15_
- [x] 2. Write the complete grammar `spec/grammar/neuron.ebnf`
  - _Requirements: R12_
- [x] 2b. Implement lexer, AST and parser (`crates/neuron-syntax`); every complete
  program in the spec parses (plan Task 11)
- [ ] 3. Review the spec with Hell and Martin; resolve comments (human; the same review
  closes `neuron-corpus` task 8)
- [x] 4. Write the validation corpus in `examples/corpus/` (A13: 50 programs across the
  eight dialects). Topics not covered yet: UART echo, Modbus slave, CAN node, I2C driver,
  stepper, 2oo3 voting, latch+ACK crossing, a five-device cell, character LCD menu; they arrive with the `proto`,
  `drivers` and `safety` layers and with phases C and D.
  - _Requirements: R12 (usability check)_
- [ ] 5. Freeze the grammar and tag `spec-v1.0`. The release candidate `spec-v1.0-rc1`
  is tagged (gate G1); the final tag waits for task 3.
- [x] 6. Write `docs/errors/` long-form explanations for the NE catalogue (`syntax.md`,
  `analysis.md`, `targets.md`, `hdl.md`, `reserved.md`; checked by `catalogue.rs`)
- [x] 7. Formatter rules and property P-FMT: written as chapter
  `spec/language/13-formatting.md` instead of `spec/format.md`
