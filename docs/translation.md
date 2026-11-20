# Translating between Neuron and other languages

Neuron programs can be emitted as the native language of other devices, and code written
in other languages can be imported as Neuron source. Both directions report how faithful
the translation is. Nothing is dropped silently.

This is a design aid. The emitted code is checked against the reference interpreter and
against virtual controllers, not against real vendor tools or hardware (see
[Open items](#open-items-to-confirm-on-real-hardware)). A hardwired E-STOP stays the
authority; the generated code is not certified for any safety standard.

## Commands

```text
neuron emit <st|scl|plcopen|spel|python|kernel> <file|dir>... [-o dir]
            [--profile p] [--robot name|path.toml] [--policy exact|equivalent|approximate]
neuron import <st|scl|plcopen|spel|c|verilog> <file> [-o out.neuron]
```

### `neuron emit`

The inputs are analyzed first, exactly as `neuron check` does, and the diagnostics are
printed the same way. The command stops with exit code 1 when the program has errors.
Then the emitter runs and the package is written to `-o` (default `out/<language>`):
the generated files, `fidelity.json`, `wiring.md` and a `<file>.map.json` source map for
each generated file that maps lines back to `.neuron` spans.

| Option | Meaning |
|---|---|
| `-o dir` | Output folder (default `out/<language>`). |
| `--profile p` | Dialect profile. `st` accepts `generic` (default) and `yaskawa`. |
| `--robot r` | Robot manifest for `spel`: `generic_6axis` (default), `generic_scara` or the path of a `.toml` file. |
| `--policy c` | The worst fidelity class the project accepts. Any entry worse than `c` is an `NE1003` error, nothing is written and the exit code is 1. |

Exit codes: 0 success; 1 the program has errors, an emitter diagnostic (`NE0610`,
`NE0804`, `NE1004`) or the policy refused the translation; 2 a usage or I/O error
(unknown emitter, unknown option, `--ros2` given to `neuron emit` instead of to the generated
program, unreadable file, unwritable folder).

Examples:

```text
neuron emit st examples/corpus/plc/p06_pump_with_levels.neuron -o out/st
neuron emit st examples/corpus/plc/p06_pump_with_levels.neuron --profile yaskawa -o out/yaskawa
neuron emit spel examples/corpus/robot/r02_palletize.neuron --robot generic_scara
neuron emit st examples/corpus/plc/p03_on_delay.neuron --policy exact   # NE1003: the timer is "equivalent"
neuron emit kernel examples/corpus/mcu/m06_traffic_light.neuron -o out/kernel
```

A diagnostic shows the file, line and source line of the construct:

```text
error[NE0804]: <what is outside the manifest>
 --> r02_palletize.neuron:<line>:<column>
  |
  | <the source line>
  = help: move the point, change the frame, or use another robot manifest (`--robot`)
```

### `neuron import`

Reads one file and prints the Neuron source on standard output. With `-o out.neuron` it
writes the source to that file and the fidelity report to `out.fidelity.json` next to it.
The source is formatted as `neuron fmt` would leave it and goes through the normal
analyzer (`neuron check out.neuron`).

`NE1002` warnings (a construct that was not imported, with its location) are printed to
standard error and do not change the exit code; the rest of the file is still imported
and a `# not imported (NE1002)` comment marks each gap. `NE1001` (the input cannot be
parsed) stops the import with exit code 1 and writes nothing. The module name is the file
name without its extension.

```text
neuron import st out/st/p06_pump_with_levels.st -o out/imported/p06.neuron
neuron check out/imported/p06.neuron
```

## Fidelity classes and policy

Every construct that is not translated exactly gets an entry in `fidelity.json` with its
location, class, and a note that states the difference and its bound.

| Class | Meaning |
|---|---|
| `exact` | Same observable behavior by construction. |
| `equivalent` | Same behavior up to a documented difference (for example one scan later). |
| `approximate` | Close; the note quantifies the difference. |
| `modeled` | A black box with a declared contract. |
| `unsupported` | Cannot be represented; reported and not translated. |

The classes are ordered from best to worst. `--policy c` accepts nothing worse than `c`.
`neuron explain NE1003` describes the diagnostic.

## Emitters

| Emitter | Reads | Writes |
|---|---|---|
| `st` | `plc` modules | IEC 61131-3 Structured Text: `<module>.st`, `NeuronLib.st` (helper blocks), and `config.st` (generic profile) or `GlobalVars.st` (`yaskawa` profile) |
| `scl` | `plc` modules | Siemens-style SCL: a function block, a cyclic interrupt organization block, `NeuronLib.scl` and `PLC_tags.csv` |
| `plcopen` | `plc` modules | PLCopen XML (TC6 2.01): LD for modules of rungs, SFC for a module that is one sequence, otherwise an ST body |
| `spel` | `robot` modules | A SPEL+-style program `<module>.prg`, `NeuronLib.prg` and the command-interpreter slave `NeuronSlave.prg` |
| `python` | `host` modules | Python 3.12 (standard library only) and the runtime `neuron_rt.py` |
| `kernel` | `fsm` and `sequence` | A state table (`<module>.nst.json`, `.nst.hex`) and four table-independent kernels: `neuron_kernel.st`, `nrt_kernel.c`, `nrt_kernel.h`, `neuron_kernel.spel` |

Output is byte-identical across runs.

### PLC languages (`st`, `scl`, `plcopen`)

| Neuron | Structured Text |
|---|---|
| `scan 10 ms` | `CONFIGURATION` with `TASK ... (INTERVAL := T#10MS)` (generic profile) |
| `inputs:` `x: bool @ %I0.0` | `VAR x AT %IX0.0 : BOOL; END_VAR` |
| `memory:` with `retain` | `VAR RETAIN` |
| `let t = ton(preset: 3 s)` | `t : TON;` called once at the end of the scan |
| `x = e` | `x := e;` |
| `set x when c` / `reset x when c` | `IF c THEN x := TRUE; END_IF;` / `... FALSE` |
| `sequence` | a `CASE` machine over the wait points |
| `fsm` | a `CASE` machine over the states |
| `safety: safe_state` | at the top of the scan: `IF neuron_fault THEN <safe state>; RETURN; END_IF;` |
| `fault F` | `neuron_fault := TRUE; neuron_fault_code := n;` (code table in `wiring.md`) |

Scan order follows the language specification: safety first, then rungs and rules in source
order, then every standard block is called once with shadow input variables
(`jam.input = x` becomes `jam_IN := x;` and `jam(IN := jam_IN, PT := T#3S);`), then time
advances by one scan. SCL uses `#x` for locals and `"x"` for tags, keeps its memory in the
instance data block, and lists the tags with their addresses in `PLC_tags.csv`.

- `yaskawa` profile: the same Structured Text with the global I/O in `GlobalVars.st`
  (`VAR_EXTERNAL`) and no `CONFIGURATION`; the task is configured in the vendor project.
- Timers are `equivalent`: the interpreter counts scans from the first true scan and an IEC
  timer measures wall-clock time, so `Q` can rise one scan later. A flasher loop gets one
  scan longer per timer per cycle. A sequence is `equivalent` within two scans.

### Robot (`spel`)

| Neuron | SPEL+-style |
|---|---|
| `point p = joints(...) deg` | `P<n> = JA(j1, ..., j6)` with a label |
| `point p = f.xyz(x, y, z) mm` | `P<n> = XY(x, y, z, 0, 0, 0) /<local>` |
| `frame f = frame(origin, rot)` | `Local <k>, XY(...)` |
| `tool t = tool(offset, payload)` | `TLSet <k>, XY(...)` and `Weight <payload>` |
| `move ptp to p speed s %` | `Speed s`, `Go P<n>` |
| `move linear to p speed v mm/s` | `SpeedS v`, `Move P<n>` |
| `move arc to b via m` | `Arc P<m>, P<b>` |
| `blend r mm` | `CP On` (`approximate`: the radius is not expressed) |
| `p.above(d mm)` | `P<n> +Z(d)` |
| `io.write(true)` / input read | `On <bit>` / `Off <bit>` / `Sw(<bit>)` |
| `until c timeout d -> S` | `Wait <cond>, d`, then `If TW Then GoTo S` |
| `fault F` | `Error <8000 + n>` with the table in `wiring.md` |

Every constant target is checked against the robot manifest at emission (`NE0804`: joint
limits, reach, z range, joint count), including each iteration of a loop with constant
bounds. A computed target emits `Call NeuronCheck(n)` and the controller raises error 8100
for a point outside the limits. Generic manifests: `devices/robots/generic_6axis.toml` and
`generic_scara.toml`. Replace the numbers with those of your robot; they describe no
vendor's product. In the fidelity report, motion time is `equivalent` (the reference uses a
fixed 1 s per motion, a controller times it by speed).

### Python (`python`)

Runs on any machine with Python 3.12 and no packages:

```text
python <module>.py --test        # run the test blocks in simulated time
python <module>.py --trace 3s    # print the first 3 s in the format of `neuron run`
python <module>.py --realtime 10s
python <module>.py --ros2        # ROS 2 node (needs rclpy), see below
```

Translated: signals with units and priorities, tasks `every` and `on`, `when` rules with
hold, safety with `reset when`, faults, functions with loops, `f32` and checked integers,
and, since C4b, each compared with the interpreter by running the Python:

- **Blocks:** `ton`, `tof`, `tp`, `ctu`, `ctd`, `ctud`, `r_trig`, `f_trig`, `sr`, `rs` and
  `pid`, in both styles (`jam.input = x` ticked by the module's 1 ms `blocks` activity, and
  `jam.call(x)`), with the interpreter's tick order and arithmetic. A `pid` whose `out`
  range has a unit stays `NE0610`.
- **State machines and sequences:** `fsm` (entry, exit, `do`, `when`/`after` transitions,
  `every` or `on` triggers, `state_of`), `sequence` (steps, `wait`, `until ... timeout -> Step`,
  `-> Step` jumps, `for`/`while`/`loop` that wait, `parallel`, `let` that lives across ticks,
  `break`, `continue`, `return`). A state machine **variable** is `NE0610`: the interpreter
  reads it as an empty value inside the state blocks, so no program can rely on it. `match`
  inside a sequence and robot `move` are `NE0610`.
- **Data:** `struct` (defaults, copy by value), `enum` with payloads, `match` (enum, `none`,
  `Ok`/`Err`, tuple, range, literal and `_` patterns, guards), `Ok`, `Err`, `none`, `?`,
  `is_some`, `is_none`, `is_ok`, `is_err`, `unwrap_or`.
- **Contracts:** `requires` and `ensures` with `result` and `old(...)` fault with
  `ContractViolation` like the interpreter.

`NE0610` (not translated): `class` and `interface`, interrupts, `io`, lambdas, string
interpolation, `mut` parameters and arguments, methods, generic functions, `??`, bit
vectors, `use` of another module and `Some(...)` (the language has no `Some`).
All translated constructs are `exact`, plus one `equivalent` entry per module (wall-clock
mode and the interpreter's statement budget are not reproduced) and one for `--ros2`.

**ROS 2 wrapper (`<module>_ros2.py`).** One node named after the module. Every `signal` is
published on `neuron/<module>/<signal>` when it changes (`Bool`, `Int64`, `Float64` or
`String` from `std_msgs`, quantities in base units); every input the program reads is
subscribed on `neuron/<module>/in/<input>`; every event a task waits for that the program
never raises itself is subscribed on `neuron/<module>/event/<event>` (`Bool`, `true`
raises it). A timer fires every 1 ms of wall time and advances the program by one base
tick. `rclpy` and `std_msgs` are imported when the node starts, so the other modes never
need ROS 2. **Verified against a stub `rclpy` only** (`crates/neuron-emit/tests/c4b_python.rs`,
`ros2_wrapper_publishes_the_interpreter_signal_changes`): the published messages equal the
interpreter's signal changes in order and in the tick they happen. It has not run on a
real ROS 2 install; confirm it in session I4 (Raspberry Pi 5).

### Kernel (`kernel`)

A state table lets a device change an `fsm` or `sequence` without a new program: the table
is data, and a fixed kernel runs it. The word encoding has a 12-word header (magic, version,
counts, checksum), variables, states, transitions and a postfix program of 16-bit values.
The checksum (`sum of word[i] * (i + 1)` modulo 65521) detects any single flipped bit.

Limits (`NE1004`): 64 states, 128 transitions, 256 program words, 32 variables, 32 inputs,
32 outputs, 16 stack values, 16-bit constants, durations up to 32767 scans.
`NE0610` (cannot be expressed): real numbers, `if`, `match`, loops and `let` inside an action
or sequence, `parallel`, `fault`, `emit`, robot motions, device `run`, edge reads
(`.rising`), state machines that run `on` an event or with an `offset`, non-constant
durations or initial values, `/`, `%` and bit operators. Of the corpus, only `m06` and `p09`
compile. A sequence is `equivalent` (within one scan of the interpreter), 16-bit wrapping
arithmetic is `equivalent` (the interpreter faults on overflow), and several writes to one
output in one scan report the last.

## Importers

| Language | Reads | Writes |
|---|---|---|
| `st` | Structured Text, generic or `yaskawa` style, hand-written or emitted | a `plc` module |
| `scl` | the SCL of the emitter | a `plc` module |
| `plcopen` | PLCopen XML with LD, SFC or ST bodies | a `plc` module |
| `spel` | the SPEL+-style subset of `neuron-spel` | a `robot` module |
| `c` | integer functions with bounded loops | a `core` module |
| `verilog` | synchronous modules with `assign` and `always @(posedge clk)` | an `hdl` module |

How the constructs come back:

| Input | Neuron |
|---|---|
| `VAR x AT %IX0.0` / `%QX` / `%MX` | `inputs`, `outputs`, `memory` |
| `TON`, `TOF`, `CTU`, ... instances | `let x = ton(preset: ...)` |
| `IF c THEN x := TRUE; END_IF` | `set x when c` |
| `IF c THEN x := e; END_IF` | `x = e when c` |
| LD networks | series contacts `and`, parallel branches `or`, negated contacts `not` |
| a `CASE` machine, or an SFC | `sequence` with `until`, `wait` and `->` |
| C `+`, `-`, `*` | `+%`, `-%`, `*%` (C unsigned arithmetic) |
| C `for` over a constant range | `while` with its trip count as the bound |
| Verilog `always @(posedge clk)` | registers of an `hdl` circuit |
| SPEL+ `Go`, `Move`, `Arc` | `move ptp`, `move linear`, `move arc` |

Fidelity of what is imported: every timer is `equivalent` (IEC timers measure wall-clock time,
Neuron timers count scans); a sequence is `equivalent` (up to two scans); an ST program without
a safe state gets `false` or `0` for each output (`approximate`); a source without a `TASK`
interval assumes 10 ms (`equivalent`); `CP On` becomes `blend 10 mm` (`approximate`); in C,
`long` is 64 bits and plain `char` is `i8` (`equivalent`); in Verilog the clock frequency
(not in the source) is declared as 100 MHz (`equivalent`).

### Not imported (`NE1002`)

Each item below is a located warning with an `unsupported` entry; the rest of the file is
still imported.

- **ST, SCL, PLCopen:** `fsm` and `when` rules of the Neuron emitter, helper blocks other
  than `hysteresis`, `WHILE`, `REPEAT`, `CASE` and `EXIT` outside the sequences the emitter
  writes, strings, arrays, edge contacts of LD, user function blocks, and non-constant
  presets.
- **SPEL+:** `If` blocks other than `If TW Then GoTo`, points with an orientation or without
  a local. Input and output bits get the names `din<n>` and `dout<n>`.
- **C:** pointers, arrays, `struct`, `union`, `enum`, floating point, strings, `goto`,
  `switch`, `do`, `while` and `for` without a bound, recursion, `void` functions, mutable
  globals, function-like macros, conditional compilation, a call to a function that was not
  imported, narrowing or sign-changing conversions, mixed signed and unsigned operands.
- **Verilog:** latch-inferring combinational blocks, blocking assignments in clocked blocks,
  non-blocking assignments in combinational blocks, asynchronous resets, several clocks,
  `initial`, tasks, functions, `generate`, module instances, memories, `integer`, bit and
  part-select assignments, `x` or `z` digits, signed literals, and arithmetic whose result is
  wider than its operands.

A syntax error in any language is `NE1001`.

## What was verified

| Check | Result |
|---|---|
| ST and SCL against the reference interpreter, in the virtual PLC of `neuron-st`, all `plc` corpus programs | exact with scan-counted timers; with IEC wall-clock timers every change within one scan, sequences within two |
| SPEL+ in the virtual controller `neuron-spel`, six robot programs in nine runs | 68 motions and 23 output changes equal (instants, kinds, targets within 1e-6) |
| Python 3.12 against the interpreter, nine programs | 168 trace lines byte-equal; 8 `test` blocks give the same `PASS` or `FAIL` |
| Python 3.12, blocks, `fsm`, `sequence`, data types and contracts, six more programs | 655 trace lines byte-equal; 20 `test` blocks give the same `PASS` or `FAIL` |
| Python `--ros2` wrapper against a stub `rclpy` | published messages equal the interpreter's signal changes in order and tick; **not run on real ROS 2** |
| Importers, round trip by traces | ST 22 programs (518 output changes), SCL 11 (259), PLCopen 11 (259), SPEL+ nine runs (68 motions, 23 changes), all equal |
| C importer | 16 functions on 572 input rows equal to the host `gcc` result and to a Rust reference |
| Verilog importer | six `hdl` programs, 3600 cycles, 4200 output values equal; re-emitted Verilog equal |
| Kernels (ST, C99, SPEL+) against the reference executor | `m06`, `p09` and two more programs on 5 random schedules each; 12 + 60 random tables in the ST and C99 kernels; every flipped bit of a table is refused |
| Command line (`crates/neuron-cli/tests/c7_translate.rs`) | every emitter and importer, the policy, `NE0610`, `NE0804`, `NE1001`, `NE1002`, round trips followed by `neuron check` |

## Open items to confirm on real hardware

- **Session I5 (PLCs):** import of the `yaskawa` profile into the vendor tool (`GlobalVars.st`,
  `VAR_EXTERNAL`, no `CONFIGURATION`), SCL and PLCopen XML import into the vendor IDEs,
  the ST kernel on a PLC, and the download of tables by the Edge Daemon.
- **Session I6 (robot arms):** SPEL+ dialect details (power-up, the `/L` suffix, reading a
  point in `NeuronCheck`), the network statements of `NeuronSlave.prg`, the contents of
  `NeuronLib.prg` (checked only as text), and the SPEL+ kernel with numeric outputs
  (verified on boolean ports only).
- **Session I4 (Raspberry Pi 5):** the ROS 2 wrapper of the Python emitter on a real ROS 2
  install (verified against a stub `rclpy` only).
- **Not done:** `class`, `interface`, `mut` parameters, string interpolation, lambdas and bit
  vectors in the Python emitter, a C emitter for `core` functions (so no Neuron to C to
  Neuron round trip).

Vendor names appear only to describe the language dialect an emitter targets.
