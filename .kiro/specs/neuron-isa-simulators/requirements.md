# Requirements: MCU instruction-set simulators (task B6)

Source: `docs/roadmap.md` task B6. The virtual devices of the digital twin (F7) run the
firmware that the tool builds, so that the lab works without hardware.

## User stories

- As a judge without a board, I want the firmware for the UNO to run in a simulated
  chip and blink the virtual LED at the same times as on the real board.
- As a maintainer, I want the compiled firmware (not only the C on the host) checked
  against the interpreter, so that a mistake of the AVR port or of the compiler flags
  is found before the hardware gate G2.

## Acceptance criteria (EARS)

1. THE AVR simulator SHALL execute the Intel HEX images that `neuron build` produces for
   the ATmega328P, counting cycles as the datasheet tables of the AVR core give them.
2. THE AVR simulator SHALL model the peripherals the runtime uses: GPIO ports B, C, D,
   `INT0`/`INT1`, timer 0 in CTC mode, the PWM compare registers of timers 1 and 2, the
   ADC, UART 0 transmit, the watchdog and sleep.
3. WHEN the firmware of a program runs in the AVR simulator THE output pins SHALL change
   in the same millisecond as the outputs of the interpreter (property MOC-A1, also on
   random inputs).
4. THE Thumb simulator SHALL execute the ARMv6-M instruction set (Cortex-M0+, the core of
   the RP2040), with SysTick, the NVIC, exception entry and return, and an RP2040-style
   GPIO block.
5. WHEN a simulator meets an encoding it does not implement, an unmapped address or an
   unaligned access THE simulator SHALL stop with an error naming the address, never
   continue silently.
6. Both simulators SHALL be deterministic: the same image and the same inputs give the
   same pin events at the same cycles.

## Out of scope for this task

Xtensa (ESP32-S3) and Cortex-M7/M55 (STM32) simulators; the RP2040 peripherals other than
SIO GPIO; the AVR instructions that the generated code does not use (`FMUL*`, `SPM`,
`ELPM`, `EIJMP`, `EICALL`), which report an error.
