# Requirements: native AVR backend (task B8)

Source: `docs/roadmap.md` task B8 ("blink without an external compiler on at least one board").

## User stories

- As a judge on a clean machine, I want `neuron build --native` to produce UNO firmware with
  nothing else installed.
- As a maintainer, I want the native image held to the same semantics as the interpreter and
  the C runtime, checked without the board.

## Acceptance criteria (EARS)

1. `neuron build --native` SHALL produce Intel HEX for the ATmega328P from MIR without any
   external tool, and an assembly listing with source lines.
2. THE native image SHALL follow the scheduling rules of the C runtime (1 ms tick, commits,
   releases, priorities, run to completion, `fsm` as periodic activities, faults to the safe
   state).
3. WHEN the image runs in `neuron-sim-avr` its pins SHALL change in the same millisecond as
   the outputs of the interpreter, also on random inputs (property MOC-N1).
4. Checked additions and subtractions SHALL fault exactly when the result does not fit.
5. WHEN a program uses a construct outside the native subset THE tool SHALL report
   `NE0610` and point to the C backend.
6. A relative branch out of range SHALL be an assembler error, never wrong code.

## Out of scope

Floats, PWM, ADC, UART, user functions, multiplication, 32-bit integers, boards other than
the UNO, RV32IMC and Thumb-2 native backends.
