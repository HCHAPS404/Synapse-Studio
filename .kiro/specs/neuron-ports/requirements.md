# Requirements: runtime ports and flash formats (tasks B4, B5)

Source: `docs/roadmap.md` tasks B4 and B5.

## User stories

- As a student with a Pico, a NUCLEO board or an ESP32-S3, I want the same blink, UART and
  interrupt program that runs on the UNO to build for my board.
- As a maintainer, I want every port checked without the board, so that hardware sessions
  only confirm what the simulator already showed.

## Acceptance criteria (EARS)

1. THE runtime SHALL have ports for the RP2040 and RP2350 (bare metal, image in SRAM), the
   STM32F4 (bare metal, image in flash) and the ESP32-S3 (ESP-IDF), with GPIO, edge
   interrupts, the 1 ms tick and UART transmit.
2. WHEN a board has a bare-metal Arm port THE `neuron build` command SHALL produce an ELF,
   a binary and, for the RP2040 and RP2350, a UF2 file written by the tool itself.
3. WHEN a board uses ESP-IDF THE tool SHALL write an `idf.py` project and build it when
   ESP-IDF is installed, and say so when it is not.
4. THE RP2040 firmware SHALL run in `neuron-sim-thumb` and drive its pins at the same
   milliseconds as the interpreter (blink, button interrupt, state machine, signal, UART).
5. `neuron flash` SHALL write the board with avrdude (AVR), the UF2 drive (RP2040,
   RP2350), probe-rs, OpenOCD or st-flash (STM32) and esptool (ESP32-S3).
6. WHEN a program uses PWM or ADC on a port that lacks them THE build SHALL stop with
   `NE0610`.
7. Every board file SHALL state its tier: `runtime`, `sdk` or `described`.

## Out of scope

The PLL at full speed (the ports run on the crystal or the internal oscillator), PWM and
ADC on the Arm ports, the CYW43 LED of the Pico 2 W, the N657 boot from external flash,
I2C and SPI on the Arm ports.
