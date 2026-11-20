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
| C3.1 | Robot manifests, `NE0804` | T4 | open |
| C3.2 | SPEL+ emitter and slave program | T4 | open |
| C3.3 | `neuron-spel` virtual controller and differential tests | T4 | open |
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
