# Tasks: MCU instruction-set simulators (B6)

- [x] 1. `neuron-sim-avr`: instruction set, flags, cycle counts, Intel HEX loader with
      checksums; unit tests.
- [x] 2. AVR peripherals used by the runtime: GPIO, `INT0`/`INT1`, timer 0 CTC, PWM
      compare, ADC, UART 0 TX, watchdog, sleep.
- [x] 3. MOC-A1: firmware built by avr-gcc from the corpus (m01, m02, m06, m07, m08, m11)
      runs in the simulator and matches the interpreter, plus random presses (property).
- [x] 4. `neuron-sim-thumb`: ARMv6-M instruction set, exceptions, SysTick, NVIC, SIO GPIO;
      tests with hand-assembled programs and a flags property test.
- [x] 5. Run RP2040 firmware built from Neuron in the Thumb simulator: the RP2040 port
      of B5 built with `arm-none-eabi-gcc` runs from a RAM image and its pins match the
      interpreter (`crates/neuron-codegen-c/tests/b5_ports.rs`).
- [ ] 6. Expose both simulators to the digital twin as virtual devices (F7, with Martin,
      once the engine WASM API exists; tracked in Phase F, not in Phase B).
- [x] 7. Evidence entry in `docs/kiro-log.md`.
