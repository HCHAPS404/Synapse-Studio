# Requirements: translation between languages (Phase C, tasks C1 to C6)

Status: v1.0 (2026-11-19). Parent spec: `neuron-language`. Normative text:
`spec/language/11-interchange.md` (fidelity report, source maps, importer and emitter
contracts) and `spec/language/14-computation-models.md` (scan, motion and reactive
models). Roadmap rows C1 to C6.

## Context

Phase B compiles Neuron to devices that Neuron controls end to end (microcontrollers,
FPGAs). Phase C covers the devices that keep their own language: PLCs (IEC 61131-3),
robot controllers (SPEL+-style) and Linux hosts (Python). It also brings existing code
into Neuron. Every translation must say what it preserved, and it must be checked by
running both sides, not by reading the output.

## Requirements

| Id | Requirement | Acceptance |
|---|---|---|
| T1 | An emitter framework: emitters are plugins with a capability manifest, are pure and deterministic, and return files, a wiring report, a fidelity report and source maps (11.4, 11.5, 11.6) | the same NIR gives byte-identical output; every construct of the input appears in the fidelity report or the output; JSON is canonical |
| T2 | A fidelity policy: a project can forbid translations worse than a class | `NE1003` names each entry that breaks the policy and nothing is written |
| T3 | `plc` modules to IEC 61131-3 Structured Text (generic profile and a Yaskawa MotionWorks-style profile), Siemens-style SCL, and PLCopen XML (LD for rungs, SFC for sequences) | the emitted ST runs in a virtual PLC and gives the same output trace as the reference interpreter for every `plc` program of the corpus, exactly with scan-counted timers and within the bound written in the fidelity report with wall-clock timers; PLCopen XML is well formed and reads back to the same program |
| T4 | `robot` modules to a SPEL+-style program and a command-interpreter slave; every constant target is checked against the robot manifest before code is written (`NE0804`), and targets computed at run time get a run-time check | the emitted program runs in a virtual controller and gives the same motion and I/O trace as the interpreter for every `robot` program of the corpus; an unreachable point is `NE0804` |
| T5 | `host` modules to Python 3 with an asyncio runtime and simulated time; an optional ROS 2 node wrapper | the emitted program, run in simulated time, passes the `test` blocks of every `host` program of the corpus and gives the same signal trace as the interpreter |
| T6 | Importers: ST and PLCopen XML (LD, SFC) to `plc`, SPEL+ subset to `robot`, a C subset to `core`, a Verilog subset to `hdl`; each writes Neuron source, a fidelity report and diagnostics (`NE1001` syntax, `NE1002` no Neuron equivalent) | round trip by traces: original Neuron, emitted, imported back, analyzed and run gives the same observable trace; hand-written inputs import with a fidelity entry for each construct |
| T7 | A micro-kernel: `fsm` and `sequence` compile to a state table (`.nst.json` and a compact word encoding) run by small interpreters for PLC (ST), MCU (C99) and robot (SPEL+), so the daemon can replace a sequence without the vendor IDE; a table beyond the kernel limits is `NE1004` | the Rust reference executor of a table gives the same trace as the interpreter for the corpus state machines and sequences; the ST kernel in the virtual PLC and the C kernel built with the host compiler give the same trace as the reference executor |
| T8 | Command line: `neuron emit <st\|scl\|plcopen\|spel\|python\|kernel> <file\|dir>... [-o dir] [--profile p] [--policy class]` and `neuron import <st\|plcopen\|spel\|c\|verilog> <file> [-o out.neuron]` | CLI tests for each command |
| T9 | No vendor material is copied; vendor names appear only descriptively; dialect details that need the real controller are marked for confirmation in the hardware sessions (I5, I6) | review of the outputs and of `docs/translation.md` |

## Correctness criterion

For a stimulus set `S`, the observable trace of the translated program equals the trace
of the original within the tolerance stated in its fidelity report (11.6, 14.10). The
observable trace of a `plc` program is the sequence of output changes; of a `robot`
program the motion targets in order and the output changes; of a `host` program the
signal changes and the results of its tests.

## Out of scope

Vendor project files (TIA Portal `.ap*`, MotionWorks projects), URScript, RAPID, KRL,
G-code and VHDL (Phase L), full PLCopen (FBD import, IL), C with pointers or the heap,
Verilog with `always` blocks that infer latches, the daemon download of state tables
(E5), validation in the vendor tools and on the arm (sessions I5 and I6).
