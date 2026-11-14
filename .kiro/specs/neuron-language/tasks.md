# Tasks: Neuron language spec (plan Task 5)

- [ ] 1. Resolve the open questions in design.md section 8 with Hell and Martin
- [ ] 2. Write the 30-program usability corpus in `examples/corpus/`
  - Cover: GPIO blink, UART echo, ADC+filter, PWM, PID thermostat, interrupt counter,
    debounced button, stepper profile, Modbus slave, CAN node, I2C sensor driver,
    state machine traffic light, GRAFCET bottling sequence, parallel branches, timers
    (TON/TOF), counters, robot pick-and-place, FPGA PWM (hdl), FPGA counter with
    latch+ACK, PLC conveyor, 2oo3 voting, watchdog, E-STOP handling, cell with 5
    devices, mutex zone, latency-bounded link, package with generics, class/FB,
    contracts, test block.
  - _Requirements: R1-R12_
- [ ] 3. Freeze the grammar in `spec/grammar/neuron.ebnf`
  - _Requirements: R12_
- [ ] 4. Write the diagnostic catalogue skeleton in `docs/errors/` (one file per code
  family)
  - _Requirements: R12_
- [ ] 5. Write the formatter rules document (`spec/format.md`)
  - _Requirements: R12 (P-FMT)_
- [ ] 6. Review with the `spec-reviewer` agent and record outcome in docs/kiro-log.md
