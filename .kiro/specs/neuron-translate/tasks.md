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
| C4.1 | Python emitter and `neuron_rt.py`, tests in simulated time | T5 | open |
| C5.1 | ST and PLCopen importers with round trip | T6 | open |
| C5.2 | SPEL+ importer with round trip | T6 | open |
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
