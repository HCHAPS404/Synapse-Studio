# Phase B prerequisites

What has to exist before the first program runs on a real board (gate G2, 2026-11-21).
Items marked **You** cannot be done by the agent.

## Already in place (end of Phase A)

- Language, analyzer, NIR, interpreter, formatter, language server and standard library,
  tagged `spec-v1.0-rc1`.
- Specs for the first two Phase B tasks: `.kiro/specs/neuron-targets/` (B1) and
  `.kiro/specs/neuron-codegen-c/` (B2, B3).
- A reference trace for every example (`neuron run`), which the C backend must match.

## Tools to install (You)

| Tool | Why | How to check |
|---|---|---|
| A C compiler for the host (MSVC or GCC via MSYS2/MinGW) | differential tests of generated C | `cc --version` or `gcc --version` |
| `avr-gcc` and `avr-libc` | build firmware for UNO and Mega | `avr-gcc --version` |
| `avrdude` | flash the UNO and Mega | `avrdude -?` |
| Arduino UNO R3 and a USB cable | gate G2 | the board appears as a COM port in Device Manager |

Quick way on Windows: install the Arduino IDE or `arduino-cli`; it ships `avr-gcc` and
`avrdude`. Give the agent the folder of those tools if they are not on `PATH`.

## Decisions taken

- C backend first, native machine code later (B8).
- UNO R3 first for G2; other boards follow in B5.
- The runtime is C with no heap and no RTOS.

## Open decisions (You)

- Which board is the first real target if the UNO is not available (Mega 2560 or Pico).
- Whether the lab computer has the toolchain, or flashing is done from another machine.

## Known gaps carried over from Phase A

- The parser reads `u8 <` as the start of generic arguments (`docs/usability-review.md`, item 4).
- `drive()` of a PLC input needs a quoted name (item 6).
- Line breaking at 88 columns in the formatter and the independent review of the
  specification are still open.
