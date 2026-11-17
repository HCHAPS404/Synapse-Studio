# Native AVR backend (task B8)

`crates/neuron-native-avr` compiles MIR straight to ATmega328P machine code and Intel HEX.
No C compiler, assembler or linker is involved: `neuron build --native` works on a machine
that has only the `neuron` tool.

```
neuron build examples/blink_mcu.neuron --native     # build/arduino_uno_r3/app.hex and app.lst
neuron flash examples/blink_mcu.neuron --native --port COM3
```

## Pieces

| Module | What it does |
|---|---|
| `asm.rs` | Two-pass assembler for the AVR instructions the generator uses (encodings from the public instruction set summary), label resolution with range checks, Intel HEX writer. |
| `codegen.rs` | Vector table, start-up, scheduler, activities, interrupt handlers. |

The generated program follows the rules of the C runtime (`runtime/neuron-rt/src/nrt_core.c`):
a 1 ms tick on timer 0 in CTC mode; in each millisecond the signal commits, then the releases,
then the ready activities by priority (equal priorities in source order), run to completion;
`fsm` blocks are periodic activities of priority 1; a fault runs the safe state and stops
every activity.

**Registers.** `r24:r25` accumulator, `r22:r23` second operand, `r16`..`r21` scratch, `r20`
fault code, `r1` always zero. Variables live at fixed SRAM addresses (no recursion, so no
frames); the stack holds return addresses and left operands. Each activity saves its stack
pointer on entry, so a fault in the middle of an expression can leave cleanly.

**Arithmetic.** 8- and 16-bit integers and `bool`. Checked `+` and `-` fault exactly when the
result does not fit (carry for `u16`, overflow flag for `i16`, a range test for 8-bit types);
`+%` and `-%` wrap. Unsigned `/` and `%` call a shift-subtract routine; a zero divisor faults.

**Shared data.** A statement that writes data shared with an interrupt runs with interrupts
off; 16-bit reads of shared data are atomic.

## Subset

Supported: periodic tasks (offset, priority, deadline, signals with logical execution time),
`on interrupt` on INT0/INT1 (rising, falling, change), `fsm` with `when` and `after`, entry,
exit and `do`, `safe_state`, `gpio.input`/`gpio.output` (`write`, `toggle`, `read`), `if`,
`for` over a constant range, bounded `while`, `break`, `continue`, `fault`.

Not yet (`NE0610`): floats, `pwm`, `adc`, `uart`, user functions, multiplication, signed
division, 32-bit integers, event tasks, `when` rules, safety rules, `reset when`, `on fault`,
the watchdog, boards other than the UNO.

## Size

The blink of the corpus is under 1500 words (3 KB) with its scheduler; the listing
(`app.lst`) shows every instruction with its address and the source line.

## Tests

`crates/neuron-native-avr/tests/b8_native.rs` loads each image in `neuron-sim-avr` and compares
the pins with the interpreter millisecond by millisecond: blink, the example blink with its
button interrupt, the traffic light (`fsm` with timeouts), two tasks sharing a signal with a
deadline, two interrupts sharing a counter with a task, an overflow fault into the safe state,
random button presses (property test), and checked additions against Rust's `checked_add`
(property test). They need no toolchain, so they always run.

Pending hardware: flashing a native image on a real UNO (with gate G2). Later native targets
are RV32IMC (Pico 2 W) and Thumb-2.
