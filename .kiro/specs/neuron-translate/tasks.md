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
| C5.3 | C subset and Verilog subset importers | T6 | open |
| C6.1 | State tables, reference executor, word encoding, `NE1004` | T7 | open |
| C6.2 | ST, C99 and SPEL+ kernels with differential tests | T7 | open |
| C7 | CLI `emit` and `import`, `docs/translation.md`, roadmap, README, kiro-log | T8, T9 | open |

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
