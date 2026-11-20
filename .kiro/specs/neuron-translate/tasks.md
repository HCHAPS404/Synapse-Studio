# Tasks: translation between languages (Phase C)

| Id | Task | Requirements | Status |
|---|---|---|---|
| C1.1 | `neuron-emit` crate: `Emitter`, `Package`, `Fidelity`, `SourceMap`, `Policy`, line writer, canonical JSON | T1, T2 | done |
| C1.2 | Diagnostics family NE10xx in chapter 10 and `docs/errors/translate.md` | T2, T6, T7 | done |
| C2.1 | `neuron-st`: ST lexer, parser and virtual PLC with both clock modes | T3 | done |
| C2.2 | ST emitter (generic and yaskawa profiles) with exact and bounded differential tests over the corpus | T3 | done |
| C2.3 | SCL emitter and tag table | T3 | done |
| C2.4 | PLCopen XML emitter (LD, SFC, ST bodies) | T3 | done |
| C3.0 | Reference interpreter runs robot programs as written: geometry values, waits inside loops, sequence locals, `MOVE` trace (spec 17) | T4 | done |
| C3.1 | Robot manifests, `NE0804` | T4 | done |
| C3.2 | SPEL+ emitter and slave program | T4 | done |
| C3.3 | `neuron-spel` virtual controller and differential tests | T4 | done |
| C4.1 | Python emitter and `neuron_rt.py`, tests in simulated time | T5 | done |
| C5.1 | ST and PLCopen importers with round trip | T6 | done |
| C5.2 | SPEL+ importer with round trip | T6 | done |
| C5.3 | C subset and Verilog subset importers | T6 | done |
| C6.1 | State tables, reference executor, word encoding, `NE1004` | T7 | done |
| C6.2 | ST, C99 and SPEL+ kernels with differential tests | T7 | done |
| C7 | CLI `emit` and `import`, `docs/translation.md`, roadmap, README, kiro-log | T8, T9 | done |

Results of C2 (2026-11-19): the Structured Text of every `plc` program of the corpus gives
the same output trace as the reference interpreter in the virtual PLC with scan-counted
timers (four random input sets each); with IEC wall-clock timers every change stays within
one scan, sequences within two; a flasher cycle is one scan longer per timer, as the
fidelity report says; SCL gives the same trace as ST; PLCopen XML is well formed with LD
and SFC bodies.

Results of C3 (2026-11-19): the six programs of `examples/corpus/robot` are emitted as
SPEL+-style programs, parsed and run in the virtual controller of `neuron-spel` and in the
reference interpreter with the same input schedules, in nine runs (`r02` with the part
sensed and never sensed, `r03` with the part held at 0 ms, held at 1500 ms and never held,
so both the normal and the timeout branch of every `until` run). 68 motions and 23 output
changes are compared in order. Motion instants, motion kinds and output instants are equal
exactly (the controller uses the interpreter's 1 s per motion and a 1 ms polling grid);
resolved targets agree within 1e-6 mm or degrees (they are equal in practice, frame-local
coordinates). Faults agree: `PartLost` and `GripLost` are `Error 8001`, the handler runs
`NeuronSafe` and `Quit All`. `NE0804` is raised for a constant target outside the manifest
(joint, reach, z range, joint count), including a target inside a loop with constant bounds on
the iteration that leaves the workspace; a computed target emits `Call NeuronCheck(n)` and the
controller raises 8100 for a point outside the limits. The fidelity report of `r02` has
11 exact constructs, one `equivalent` (motion time: fixed 1 s in the reference, the controller
times by speed) and one `approximate` (`blend` becomes `CP On`). Output is byte-identical
across runs and has source maps. Dialect details that need the real controller (power-up,
`/L` suffix, reading a point in `NeuronCheck`, network statements of `NeuronSlave.prg`)
are marked "to confirm in session I6". Generic manifests: `devices/robots/generic_6axis.toml`
and `generic_scara.toml`.

Results of C4 (2026-11-19): the five `host` programs of the corpus and four more written in
`tests/c4_python.rs` (signals with units and priorities, a fault with `reset when`, an event
task, functions with loops, `f32` and checked integers) are emitted and run with Python 3.12
(`python <m>.py --trace <T>` and `--test`). The trace text is byte-equal to the reference
interpreter's in all nine runs: 168 lines in total (h01 5, h05 1, x1 41, x2 76, x3 22,
x4 23; h02 to h04 have no trace, they only run tests). The nine programs have eight `test`
blocks; Python reports the same `PASS` or `FAIL <name>: <message>` as `run_tests` for each,
including a failed assertion and an `Overflow` fault. Output is byte-identical across runs,
has source maps, uses only the standard library and is ASCII. Runtime: `neuron_rt.py`
(event queue in integer nanoseconds, logical execution time, tasks `every` and `on`, `when`
rules with hold, safety with `reset when`, `--test`, `--trace`, `--realtime`). Fidelity: all
translated constructs are exact; one `equivalent` entry per module (wall-clock mode and the
interpreter's statement budget are not reproduced), plus `watchdog` (not enforced, as in the
interpreter), `verify` and the C-library math functions when used. Not translated (`NE0610`
with location): `struct`, `enum`, `class`, `interface`, state machines, sequences,
interrupts, `io`, `match`, options and results, lambdas, string interpolation, `mut`
parameters, `ensures`, bit vectors, `timer and counter` blocks (they belong to `plc`).
`--ros2` is refused with exit code 2 and recorded as not supported in `wiring.md`; nothing
needs `rclpy`. CI installs Python 3.12 and sets `NEURON_REQUIRE_PYTHON=1`.

Addendum C4b (2026-11-20): the Python emitter now also translates, each proved by running
the emitted Python against the interpreter (`crates/neuron-emit/tests/c4b_python.rs`, exact
text equality of `--trace` and of every `--test` verdict, six new host programs, 655 trace
lines and 20 `test` blocks in total):

| Program | Constructs | Trace lines equal | Tests agree |
|---|---|---|---|
| `y1_timers` | `ton`, `tof`, `tp`, `ctu`, `r_trig`, `f_trig` (property style, 1 ms `blocks` tick) | 34 | 3 |
| `y2_methods` | `sr`, `rs`, `ctud`, `ctd`, `pid` (`out` range, `antiwindup`) with `.call(x)` | 411 | 3 |
| `y3_fsm` | `fsm` (entry, exit, `do`, `after`, `when`, `every`, `on event`), `state_of` | 83 | 3 |
| `y4_sequence` | `sequence`: `until ... timeout -> Step`, `wait`, `for`/`while`/`loop` that wait, `continue`, `break`, `-> Step` | 32 | 2 |
| `y5_types` | `struct` (defaults, copy), `enum` with payload, `match` with guards, `Ok`/`Err`/`none`, `?` | 80 | 4 |
| `y6_contracts` | `requires`, `ensures` with `result` and `old`, `ensures` after an early `?` return | 15 | 5 |

Decisions: the interpreter reads a state machine variable as an empty value inside the state
blocks, so it stays `NE0610` (no program can rely on it); `Some(...)` is not in the language
(sema says unknown name), so `Some` patterns are emitted but not covered by a test; `match`
inside a `sequence` and robot `move` are `NE0610` (bindings use `:=`, which cannot target a
sequence local). Still `NE0610`: `class`, `interface`, interrupts, `io`, lambdas, string
interpolation, `mut` parameters, methods, generic functions, `??`, bit vectors and `use` of
another module.

`--ros2` (replaces the refusal above): the emitter writes `<module>_ros2.py`, a node with one
publisher per signal (`neuron/<module>/<signal>`), one subscriber per input and per external
event, and a 1 ms timer that advances the program one base tick. The test
`ros2_wrapper_publishes_the_interpreter_signal_changes` writes a stub `rclpy` and `std_msgs`
into a temporary directory, puts it on `PYTHONPATH`, fires the timer 400 times and checks
that the published messages equal the interpreter's signal changes, in order and in the same
tick (at least 60 messages), and that messages injected on a subscription reach the program.
**Verified against a stub only**; confirm on a real ROS 2 install in session I4 (Raspberry
Pi 5). The fidelity report has an `equivalent` entry `ros2` saying so, and `wiring.md` repeats it.

Results of C5.1/C5.2 (2026-11-20): `crates/neuron-import` reads Structured Text (`st`), the SCL
of the emitter (`scl`), PLCopen XML (`plcopen`) and the SPEL+-style subset (`spel`) and writes
formatted Neuron source (`neuron fmt` leaves it unchanged), a fidelity report and diagnostics.
Round trip by traces (`tests/c5_import.rs`, original and imported program both run in the
reference interpreter with the C2 random schedules, five per program): ST generic and `yaskawa`
profiles, 22 programs (11 each), 518 output changes equal; SCL, 11 programs, 259 changes equal;
PLCopen XML, 11 programs (7 LD, 1 SFC, 3 ST bodies), 259 changes equal. `p09` (a sequence) comes
back as the same `sequence` with `initial step Idle`, `until`, `wait 5 s` and `-> Idle`, from the
`CASE` machine of the ST, the SCL and the SFC. SPEL+: the six robot programs in the nine runs of
C3 (both branches of `r02` and `r03`), 68 motions and 23 output changes equal (instant, kind
and resolved target of each motion, instant, bit and value of each change, and the fault).
Nothing in the corpus fails to round trip. Hand-written inputs: a latch with a `TON`, a counter
with a memory variable, a SPEL+ cell, a `WHILE` loop (one `NE1002` with the span of the loop and
one `unsupported` entry; the rest is imported and a `# not imported (NE1002)` comment marks the
gap), a `CASE`, a user function block and a non-constant preset (one `NE1002` each), and a
syntax error in each language (`NE1001` with the position). Not exact, as the reports say:
every timer is `equivalent` (an IEC timer measures wall-clock time, a Neuron timer counts scans),
a sequence is `equivalent` (up to two scans), an ST program without a safe state gets
`false` or `0` for every output (`approximate`), a source without a `TASK` interval assumes 10 ms
(`equivalent`), `CP On` becomes `blend 10 mm` because the radius is not in the program
(`approximate`). Not read back yet (each one is `NE1002`): `fsm` and `when` rules of the Neuron
emitter, the helper blocks other than `hysteresis`, `WHILE`/`REPEAT`/`CASE`/`EXIT`, strings and
arrays, edge contacts of LD, SPEL+ `If` blocks other than `If TW Then GoTo`, points with an
orientation or without a local. SPEL+ input and output bits get the names `din<n>` and `dout<n>`
(the names are only in the wiring report); labels, frames and tools keep the names of the
comments the emitter writes.

Results of C6 (2026-11-20): `neuron-emit/src/kernel/` (`mod.rs` model, word encoding and
reference executor `Exec`; `compile.rs` NIR to table; `emit.rs` the `kernel` emitter, registered
in `names()` and `emitter()`) and four table-independent kernel texts included as files:
`neuron_kernel.st` (`FUNCTION_BLOCK NEURON_KERNEL` over `ARRAY [0..1023] OF INT`),
`nrt_kernel.c` and `nrt_kernel.h` (C99), `neuron_kernel.spel` (`NeuronKernel`, `NkRun`,
`NkInit`, `NkWrap`). Word encoding: 12 header words (magic `0x4E53`, version 1, flags, counts,
initial state, program length, total length, checksum = sum of `word[i] * (i + 1)` mod 65521,
which detects any single flipped bit), variables, 5 words per state, 3 per transition, program
of postfix blocks over 16-bit values; the text form `.nst.hex` is four hex digits per word.
Limits (`NE1004`, documented in `docs/errors/translate.md`): 64 states, 128 transitions, 256
program words, 32 variables, 32 inputs, 32 outputs, 16 stack values, constants of 16 bits,
durations up to 32767 scans. Workspace: 502 tests pass (baseline 492); `cargo fmt --check`, clippy
with `-D warnings` and `cargo test --workspace` with `NEURON_REQUIRE_PYTHON=1` end with exit 0.
Tests (`tests/c6_kernel.rs`, 10 tests, about 30 s):

| Program | Table | (a) executor vs interpreter, 5 random schedules of 12 s | ST kernel | C99 kernel | SPEL+ kernel |
|---|---|---|---|---|---|
| `m06_traffic_light` (fsm, `mcu`) | 81 words, 3 states, 3 transitions | 35 output changes and 20 state changes, exact (0 ms) | 35 equal | 35 equal | 35 equal |
| `p09_sfc_mixer` (sequence, `plc`) | 94 words, 5 states, 5 transitions | 44 changes, within one scan (worst 10 ms, the kernel is a scan earlier) | 44 equal | 44 equal | 29 equal (boolean outputs, no first `false`) |
| `c6_door` (fsm, inputs, variable, `do`, `when`/`after`) | 104 words, 3 states, 4 transitions | 2524 changes and 2149 state changes, exact | 2527 equal | 2527 equal | 2522 equal |
| `c6_press` (sequence, timeout, `->`, counter, integer output) | 146 words, 6 states, 7 transitions | 209 changes, within one scan (worst 10 ms) | 209 equal | 209 equal | 144 equal (boolean outputs) |

"Equal" means the same scan, output and value in the same order as the reference executor over
5 schedules of 1200 scans. Beyond the four programs: 12 random tables x 150 scans in the ST
kernel (304 output changes) and 60 random tables x 400 scans in the C99 kernel built with the
host `gcc` (`-std=c99 -pedantic -Wall -Wextra -Werror`; 2710 output changes) equal the executor,
so the kernels do not depend on the table; a property test (48 random tables) checks
`decode(encode(t)) == t`, the hex text, and that each of the 16 bits of each word, flipped one
at a time, is refused; a flipped word of a real table is refused by every word position
(`nrt_init` answers 4, a bad magic 1); a table of 70 states and a timeout of 400 s are `NE1004`;
`r01` (robot motions) and `x03` (device `run`) are `NE0610` with their spans; the emitter output is
byte-identical across runs. The fidelity of a state machine whose period equals its scan is exact;
a sequence is `equivalent` with the bound "within one scan of the reference interpreter" (the
interpreter advances a sequence every millisecond and outputs are published at the scan; with
the corpus input patterns the kernel is exactly one scan earlier for a change that follows an
input or a timer, 10 ms at a 10 ms scan). Also `equivalent`: 16-bit wrapping arithmetic (the
interpreter faults on an overflow) and several writes to one output within one scan (the
interpreter traces each write; the table reports the last).
What the kernel cannot express (`NE0610`, with the location): real numbers, `if`, `match`, loops
and `let` inside an action or a sequence, `parallel`, `fault`, `emit`, robot motions, device `run`,
edge reads (`.rising`), state machines that run `on` an event or with an `offset`, durations or
initial values that are not constants, `/`, `%` and bit operators. Of the corpus programs only
`m06` and `p09` can be compiled; the other `fsm` and `sequence` items (six robot programs, `x03`,
`bottling_line`, `pick_robot`) use motions or device actions.
Deviations: (1) the SPEL+ kernel needed arrays: `neuron-spel` now accepts `Integer t(100)`,
`t(i) = x` and `t(i)` (global arrays of numbers) and its tests are unchanged; the controller's
numbers are floating point, so the kernel wraps `+ - * -x` with a function (`NkWrap`) and the
differential test covers only boolean outputs (`On`/`Off`) and inputs (`Sw`). (2) `neuron-st`
evaluates `a[i]` of a variable in place instead of copying the array (needed for a table of a
thousand words); behaviour is unchanged. (3) The checksum is verified by the three kernels at
the first scan; the kernels trust the structure of a checksummed table (the host `Machine::check`
validates it). (4) Not done: CLI (`neuron emit kernel` is task C7), C source for a table
(`.nst.h`), kernels in other languages.

Results of C5.3 (2026-11-20): `neuron import c` (`crates/neuron-import/src/c.rs`) and
`neuron import verilog` (`src/verilog.rs`) are hand-written parsers without new dependencies;
`tests/c53_import.rs` has 14 tests (workspace total 492, baseline 478).
C: a 16-function file (`gcd`, `lcm` calling `gcd`, `fib`, saturating `sat_add` and `sat_add8`,
`clamp`, `rotl`, `popcount`, `parity`, `hash8`, `mix`, `abs_diff`, `in_range`, `sign`, `sum_to`,
`first_multiple` with `continue` and `break`, `#define` and `const` globals, bound comments and
constant-range `for` loops) is imported with no diagnostics, analyzed, and every function is run
in the reference interpreter through generated `test` blocks on 572 input rows. The results equal
those of the same C compiled with the host `gcc` (w64devkit, `NEURON_CC`) in all 572 rows, and
equal a Rust reference in the same 572 rows. Semantics: `+`, `-`, `*` become `+%`, `-%`, `*%`
(C unsigned arithmetic), `<<` on an unsigned type becomes `*%` by a power of two, a `for` becomes
a `while` with its trip count as the bound (Neuron's `for v in a..b` gives `v` the type `index`,
which does not mix with `i32`), a parameter that the body assigns is copied to a local, and
conversions are written only when lossless. The `neuron emit-c` output (`blink_mcu.c`) is
firmware (runtime tables, structs, pointers, `nrt.h`) and there is no C emitter for `core`
functions yet (the C99 kernel is task C6.2), so there is no Neuron to C to Neuron round trip; the
test imports that file and checks that every part outside the subset is a located `NE1002`.
Verilog: the Verilog that `neuron-codegen-hdl` writes for the six `hdl` programs (`pwm_hdl` and
`d01` to `d05`; `d02` with a half period of 5 and `d05` with a divisor of 4 so that they move) is
imported, analyzed and emitted again: the second Verilog equals the first (without its comment
lines) for all six, and a cycle simulator of the NIR circuits (no Verilog simulator is installed;
`iverilog` is not required by these tests) gives the same output waveform for the original and
the imported circuit: 6 programs, 3600 cycles, 4200 output values equal. Hand-written modules
checked against Rust models: a counter with enable and synchronous reset (400 cycles; `if (rst)`
becomes the reset value of the register, which the backend writes again), an edge detector with
`assign` (300 cycles) and a combinational ALU with `case`, a parameter and a wire (300 cycles,
two outputs). Fidelity: each circuit is `exact`; the clock frequency (not in Verilog) is declared
as 100 MHz (`equivalent`), registers without a value start at 0 (`equivalent`), a sensitivity
list is read as `@*` (`equivalent`), and a port called `rst` or `reset` that is not a plain
synchronous reset is renamed `<name>_in` because the backend adds a reset to such a port
(`equivalent`).
Not imported, each one a located `NE1002` with an `unsupported` entry (the rest is imported):
C: pointers, arrays, `struct`/`union`/`enum`, floating point, strings, `goto`, `switch`, `do`,
`while` and `for` without a bound, recursion (also mutual), `void` functions, mutable globals,
function-like macros, conditional compilation, a call to a function that was not imported (and
so every caller of a function that failed), narrowing or sign-changing conversions, mixed
signed and unsigned operands, `continue` in a `for` that is not a constant range. Verilog:
latch-inferring combinational blocks, blocking assignments in clocked blocks, non-blocking
assignments in combinational blocks, asynchronous resets and several clocks, `initial`, tasks,
functions, `generate`, module instances and primitives, memories, `integer`, bit and part
select assignments, `x` or `z` digits, signed literals, and arithmetic whose result is wider
than its operands (Neuron would drop the carry). A syntax error in either language is `NE1001`.
Deviations: `long` is imported as 64 bits (LP64) and plain `char` as `i8` (both `equivalent`);
operands narrower than `int` are not promoted (`equivalent`, one entry per file); a Verilog
output register written by a clocked block gets an internal register `<name>_r` and a `comb`
copy, so the re-emitted Verilog of a hand-written module is not identical to its source.

Results of C7 (2026-11-20): `neuron emit <st|scl|plcopen|spel|python|kernel> <file|dir>...
[-o dir] [--profile p] [--robot name|path.toml] [--policy exact|equivalent|approximate]` and
`neuron import <st|scl|plcopen|spel|c|verilog> <file> [-o out.neuron]` live in
`crates/neuron-cli/src/translate.rs` (a module of the binary; `main.rs` only dispatches and
lists the usage). `emit` analyzes like `check` (same diagnostics, exit 1 on errors), runs the
emitter, applies the policy (`NE1003`: diagnostics, nothing written, exit 1), writes the package
to `-o` (default `out/<language>`) and prints the files and the fidelity counts; emitter
diagnostics (`NE0610`, `NE0804`, `NE1004`) show file, line and source line. `--ros2` and
unknown options or emitters exit 2. `import` prints the source or writes `-o` and
`<out>.fidelity.json` (`out.neuron` gives `out.fidelity.json`), prints `NE1002` as warnings
(exit 0) and exits 1 on `NE1001` writing nothing. `crates/neuron-cli/tests/c7_translate.rs` has
15 tests that run the binary: st generic and `yaskawa`, scl, plcopen (SFC), spel with the default
robot and an `NE0804` case (a manifest with a 100 mm reach), python, kernel (`m06`), the policy
refusing `p03` (`NE1003`, no folder), `NE0610` (`r01` to the kernel), usage errors, analyzer
errors, round trips for st, scl, plcopen and spel (emit, import, `neuron check` passes), c and
verilog imports, `NE1002` and `NE1001`. `docs/translation.md` is the user guide (the target of the
`NE1001` and `NE0610` help text). CI and `tools/phase-check.ps1` have smoke steps for emit
(st, spel, python, kernel) and import (st). Workspace: 517 tests pass (baseline 502, plus the 15 of `c7_translate.rs`); `cargo fmt --check`,
clippy with `-D warnings` and `cargo test --workspace` with `NEURON_REQUIRE_PYTHON=1` end with
exit 0.

Addendum C8 (2026-11-20): the `c` emitter, `core` modules to portable C99
(`crates/neuron-emit/src/c99/`: `mod.rs`, `nrt_core.h`, `nrt_core.c`). Output per module:
`<module>.c`, `<module>.h`, plus `nrt_core.h`/`nrt_core.c`, `fidelity.json`, `wiring.md` and a
source map of `<module>.c`. `neuron emit c` is wired in `neuron-cli` (usage text updated); the
firmware command `neuron emit-c` is unchanged (`b4_cli.rs`: 13 tests green). Faults: the
interpreter's `Overflow`, `DivideByZero` and `ContractViolation` are reproduced by checked
helpers that set the global `nrt_fault` (the first fault wins), yield 0 and go on; the caller
clears the flag before a call and reads it after (fidelity `equivalent`, "checked integer
arithmetic"). Units are erased to `double` values in base SI units (fidelity entry "units").
One fix to the first draft: arithmetic keeps the type the analyzer gave it instead of taking the
type of its context (`x & ((1 << n) - 1)` with `n = 32` does not fault in the interpreter, so it
must not in C). Not translated, `NE0610` with a location: classes, interfaces, options,
results, arrays, tuples, strings, bit vectors, generics, `mut` parameters, contracts other than
`requires`, recursion (also rejected by the analyzer, NE0302), and a program without a `core`
module. Float to integer `as` does not exist in Neuron (NE0204), so the runtime's real to
integer helpers are not reachable yet.
Tests `crates/neuron-emit/tests/c8_core_c.rs` (15) build the emitted C with the host `gcc
-std=c99 -pedantic -Wall -Wextra -Werror -O2` into a generated driver and compare with the
interpreter: 13 programs (eight written in the test, `c01`, `c02`, `c05`, `std.core.bits`,
`std.core.math`), 754 calls equal, 33 of them faults that fault the same way in C
(per program: intmath 33 calls/7 faults, sat 369/3, gcd 191/2, bits 53/8, enum 13/0, struct
10/3, calls 23/6, real 22/0, c01 5/1, c02 3/0, c05 5/0, bits lib 15/2, math lib 12/1); the
round trip emitted C, `import c`, analysis, interpreter: 260 calls equal (the importer reads
the part with no checked helpers); determinism; maps and fidelity; `-Werror` build of every
program; `NE0610` for `c03`, `c04`, an array parameter, a tuple and a non-`core` module. A
missing compiler fails with `NEURON_REQUIRE_TOOLCHAIN=1` and skips otherwise.
`c7_translate.rs` has 16 tests (one new for `neuron emit c`). Workspace: 540 tests pass (baseline 524, plus 15 in `c8_core_c.rs` and 1 in `c7_translate.rs`;
doc-tests included); `cargo fmt --check`, clippy with `-D warnings` and `cargo test --workspace`
with `NEURON_REQUIRE_PYTHON=1` end with exit 0.
