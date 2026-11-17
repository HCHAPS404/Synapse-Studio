# Tasks: runtime ports and flash formats (B4, B5)

- [x] 1. Arm toolchain in `tools/install-toolchains.ps1` (xPack 15.2.1, SHA-256 checked) and CI packages.
- [x] 2. Port-neutral I/O table (`NRT_GPIO`, `NRT_PIN` redefinable), GPIO numbers from `GPIOn` pins.
- [x] 3. RP2040 / RP2350 port (start-up, crystal clock, SIO GPIO, IO_BANK0 interrupts, SysTick, UART0).
- [x] 4. STM32F4 port (start-up from flash, HSI, GPIO, EXTI, SysTick, USART2).
- [x] 5. ESP-IDF port and the `idf.py` project generator.
- [x] 6. `build_arm`, the UF2 writer, `flash_uf2`, `flash_stm32`, `flash_esptool`; CLI dispatch by port and programmer.
- [x] 7. RP2040 peripherals in `neuron-sim-thumb`; the RP2040 firmware matches the interpreter (`tests/b5_ports.rs`).
- [x] 8. Tiers in the board files (`runtime`, `sdk`, `described`) and in `docs/targets.md`.
- [ ] 9. Same blink/UART/IRQ program on each real board (session I2; needs the boards).
- [ ] 10. PWM and ADC on the Arm ports; the PLL at full speed.
