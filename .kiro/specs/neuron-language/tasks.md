# Tasks: Neuron language spec (plan Task 5)

- [x] 1. Write the language specification chapters 01–10 in `spec/language/`
  - _Requirements: R1–R15_
- [x] 2. Write the complete grammar `spec/grammar/neuron.ebnf`
  - _Requirements: R12_
- [x] 2b. Implement lexer, AST and parser (`crates/neuron-syntax`); every complete
  program in the spec parses (plan Task 11)
- [ ] 3. Review the spec with Hell and Martin; resolve comments
- [ ] 4. Write the validation corpus in `examples/corpus/` (one program per dialect
  feature: blink, UART echo, ADC + filter, PWM, PID, interrupt counter, debounce,
  stepper, Modbus slave, CAN node, I2C driver, traffic-light fsm, GRAFCET sequence,
  parallel branches, IEC timers/counters, robot pick-and-place, palletizing, FPGA PWM,
  FPGA UART TX, latch+ACK crossing, PLC conveyor, 2oo3 voting, watchdog, E-STOP,
  5-device cell, mutex zone, latency-bounded link, generics, class/FB, contracts,
  tests, TFT dashboard, character LCD menu)
  - _Requirements: R12 (usability check)_
- [ ] 5. Freeze the grammar (gate G1) and tag `spec-v1.0`
- [ ] 6. Write `docs/errors/` long-form explanations for the NE catalogue
- [ ] 7. Write `spec/format.md` (formatter rules, property P-FMT)
