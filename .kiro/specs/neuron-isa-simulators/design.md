# Design: MCU instruction-set simulators (task B6)

## Crates

| Crate | Core | Memory and peripherals |
|---|---|---|
| `neuron-sim-avr` | AVR (ATmega328P), cycle counted | 32 KiB flash, 2 KiB SRAM, I/O registers; GPIO B/C/D, `INT0`/`INT1`, timer 0 CTC, timer 1/2 compare registers, ADC, UART 0 TX, watchdog, sleep |
| `neuron-sim-thumb` | ARMv6-M (Cortex-M0+), Cortex-M0+ timings of the common cases | configurable flash and RAM (RP2040 layout by default); SysTick, NVIC (32 IRQs), `VTOR`, `PRIMASK`, SIO GPIO |

Both are plain Rust with no `unsafe`, no threads and no clock: time is the cycle
counter. Inputs are scheduled at a cycle (`schedule_pin`, `schedule_input`) and applied
when the counter passes it, which makes a run deterministic.

## Time

- The AVR simulator converts cycles to milliseconds with `F_CPU`. Sleep (and `WFI` on
  the Thumb core) skips to the next event (a timer match, a scheduled input) instead of
  stepping idle cycles, so a 10 s run of a blink takes milliseconds.
- Pin events carry the cycle of the store that changed the pin.

## Comparison with the interpreter (MOC-A1)

The interpreter traces every write of an output; a pin only changes when the level
changes. The test drops repeated levels from the interpreter trace and compares
`(whole millisecond, output, level)` lists. Float signals are not compared (the
interpreter prints them with units).

## Thumb simulator scope

The RP2040 has no C runtime port yet (B5), so the Thumb simulator is tested with
programs encoded by a small assembler in its test file: arithmetic and flags (a
property test against a 64-bit reference), loads and stores of every width, calls
through the stack, SysTick at an exact period, `PRIMASK`, an input copied to the LED,
and faults. Other peripheral addresses read as zero and ignore writes, so start-up code
that waits on a status bit of an unmodelled peripheral does not finish.
