# Hardware and target errors (tasks B1 to B4)

Source of truth for the catalogue: `spec/language/10-diagnostics.md`. The rules are in
`spec/language/07-hardware.md`; the target files are described in `docs/targets.md`.
`neuron explain NE0601` prints the matching section. These checks run in
`neuron build` (and `neuron emit-c`) after the semantic analysis, against the board
named by `module ... for mcu on <board>` or by `--board`.

## Resources and pins

### NE0601 hardware resource bound twice
A pin, a peripheral instance or a timer channel belongs to two `io` bindings, or to a
binding and the runtime. Each resource has one owner (property HW-1). The runtime of
the AVR boards owns `timer0` for its 1 ms tick, so PWM on pins of `timer0` (D5 and D6
on the UNO) is refused: use a pin of `timer1` (D9, D10) or `timer2` (D3, D11).

### NE0603 pin does not support this function
The pin does not exist on the board, or it lacks the function that the binding needs
(`pwm`, `adc`, an external interrupt for `on interrupt x.rising`). The help lists the
pins that do have it. A peripheral location such as `UART3` that the chip does not have
is reported with the same code.

## Clocks and rates

### NE0605 requested clock frequency not achievable
The `clocks:` block asks for a `sysclk` that the clock tree of the chip cannot make
from the chosen source with its PLL limits and dividers, or names a source that the
chip does not have. The help gives the nearest frequencies below and above.

### NE0606 derived rate error above tolerance
A UART baud rate or a PWM carrier, computed from the system clock with the dividers the
chip has, is further from the request than the tolerance: 3 % for UART and 5 % for PWM
unless the binding sets `tolerance`. Pick a rate that divides the clock better, change
the clock, or raise the tolerance if the other end accepts it.

## Targets

### NE0607 unknown target, chip or board
The board after `on` is not in the registry, or one of its files names a chip,
architecture or runtime that does not exist, or a target file is malformed. The message
lists the known names; `neuron targets` prints the table.

### NE0608 peripheral option beyond what the chip supports
An option of a binding asks for more than the peripheral can do, for example
`adc(bits: 12)` on a chip whose converter has 10 bits. Lower the option or use a
board whose chip supports it.

## Code generation

### NE0610 construct not supported by the selected code generator
The program is valid Neuron, but the C backend does not translate this construct yet.
The message names it (for example `match`, a class, a native function block such as
`ton`, a `sequence`, an array). The interpreter (`neuron run`) runs every valid
program; the C backend covers the subset listed in `docs/targets.md`. Nothing is
silently dropped: a program with `NE0610` produces no firmware.
