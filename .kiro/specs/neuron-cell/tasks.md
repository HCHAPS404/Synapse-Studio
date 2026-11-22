# Tasks: cell compiler (Phase D)

Order: each task ends with tests green, a `docs/kiro-log.md` entry, the roadmap row,
a Conventional Commit with `-s` and CI checked.

| Id | Task | Requirements | Needs hardware | Status |
|---|---|---|---|---|
| D1.1 | Manifest schema and loader (`devices/{plcs,controllers,fpgas,robots,hosts,media}`), trust levels, `NE0607` | D-R1, D-R11 | no (generic numbers) | done |
| D1.2 | Generic manifests for every device of the corpus cells and the bottling example; the same module on two PLC vendors | D-R1 | no | done |
| D2.1 | Partitioner: explicit `place`, automatic assignment, `NE0704`, `NE0705`, `CellPlan` | D-R2 | no | done |
| D2.2 | Per-device build through the existing backends and emitters | D-R2 | no | done |
| D3.1 | Link endpoints as NIR items, register maps, Neuron Wire frames, Latch+ACK, BUSY/DONE/TIMEOUT | D-R3 | no | open |
| D3.2 | Latency formula, `NE0701` with alternatives, link table | D-R4 | no (generic profiles; measured in I1) | done |
| D3.3 | Cell watchdog and safe state on loss | D-R5 | no | open |
| D3.4 | Tags across devices, `NE0703`, single writer | D-R6 | no | open |
| D4.1 | Zones and mutexes, `NE0805`, robot targets in zones | D-R7 | no | open |
| D4.2 | Electrical compatibility, `NE0801` | D-R7 | no | open |
| D4.3 | Cell `verify:` over the composed machines, shortest counterexample | D-R7 | no | open |
| D6.1 | Co-simulation on the cell clock, link models, `--cut`, partition equivalence | D-R8, MOC-C1, MOC-C2 | no | open |
| D6.2 | Replay format and JSON Schema for the twin | D-R8 | no | open |
| D5.1 | Dossier: Markdown and HTML with SVG timing diagram, BOM, reports | D-R9 | no | open |
| D7 | CLI `neuron cell check|build|sim|dossier`, `docs/cells.md`, CI smoke steps, roadmap, README | D-R10 | no | open |
| D-HW | Confirm timing profiles (I1), run the bottling cell on real devices (I4 to I7) | D-R1, D-R4 | **yes** | pending (sessions) |

## Results of D1.1

New crate `neuron-cell` with `manifest.rs` (device and medium schema: kind, dialects,
backend, profile, trust `generic < measured < confirmed`, I/O families with voltage
domain, timing per medium, safe-state time; unknown fields refused; units checked with
the analyzer's table), `library.rs` (`Library::builtin` embeds the files at build time,
`from_dir` and `overlay_dir` load a `devices/` tree; `check` resolves targets, FPGA
boards and media; `resolve_cell` finds every device and medium a cell names) and
`units.rs`. Every failure is `NE0607`, whose meaning now covers device manifests
(chapter 10 and `docs/errors/targets.md`); no new code. Media: `modbus_tcp`, `s7`,
`zero_hop` with generic numbers. The two Phase C robot manifests gained `[device]`,
`[io]`, `[timing]` and `[safety]` and keep `[robot]`; the C3 tests of `neuron-emit` still
pass on them. Tests: 11 (3 proptests: exact durations, parsing never panics and always
yields `NE0607`, values round trip) and 6 doctests.

## Results of D1.2

Four manifests: `plcs/generic_s7_1200` (SCL, vendor class `s7`), `plcs/generic_iec_plc`
(ST with the `yaskawa` profile, vendor class `iec_st`), `controllers/nucleo_f446re`
(C backend, points at the existing board file) and `fpgas/generic_artix7_board` (Verilog,
points at `artix7_35t_board`). All are `generic` with a note naming the session that
measures them. The four corpus cells and the bottling example (loaded as the `examples`
package, because it places modules of other files) resolve every device and medium; the
cells cover the plc, mcu, fpga and robot classes. `conveyor_plc` builds for both PLC
manifests through their own backend and profile and gives two different programs.
Modbus TCP on the NUCLEO board assumes a serial-to-Ethernet gateway (noted in the file).

## Results of D2.1

`partition.rs` turns every cell of a package into a `CellPlan` (one `DevicePlan` per
`device`: manifest, class, backend, profile, modules with how they were placed, cell
sequences). Rules decided here, since the design left them open: a module is a candidate
for automatic placement when no cell places it, it is not `core` (library code), and its
board, if it names one, is the board of a device of the cell; it goes to the device that
runs its dialect and shares the most signal and `io` names with the modules already
there, ties to the device declared first; candidates are taken in name order. A cell
`sequence` without `on` runs on the first PLC, else the first host. The bottling cell
gives four programs: `thermostat` on `ctl` and `conveyor` on `plc` (explicit), `pick` on
`arm` and `pwm8` on `fpga` (automatic), and the `Cycle` sequence on `plc`; `blink` and
`panel` are written for boards outside the cell. New codes `NE0704` (impossible
placement, including an unknown device, a module placed twice, a sequence on an FPGA)
and `NE0705` (fits no device, with what each device lacks), in chapter 10,
`docs/errors/cell.md` and `neuron explain`. Tests: 8, with a property test that the
plan does not depend on the order of the source files. Address-level capability checks
(pins and register addresses) are left to D3 and D4, where the link endpoints and
voltage domains are known.

## Results of D2.2

`build.rs` builds every `DevicePlan` with the backend of its manifest and returns a
`CellBuild` (files per device). Each device gets its own NIR package: its placed modules
in plan order plus the `core` modules of the package; a controller module without a
board takes the board of the manifest. Backends: the C path of `neuron build` (hardware
checks, MIR, C and the ui runtime), Verilog plus XDC or PCF, and the Phase C emitters for
ST, SCL, PLCopen, SPEL+ (with the robot manifest of the device) and Python, whose
`fidelity.json`, `wiring.md` and source maps are kept. Two manifests added:
`controllers/arduino_uno_r3` and `hosts/raspberry_pi_5`. A cell of corpus programs with
one device of each class (S7-class PLC, UNO, six-axis arm, Artix-7 board, Raspberry Pi 5)
builds to SCL, C, a SPEL+-style program, Verilog with XDC, and Python; the build is
deterministic. The bottling example does not build yet, and the reasons come out per
device: `thermostat` uses function blocks (`pid`, `lowpass`) that the C backend does not
support (B2 subset, `NE0610`), and `pick` is a six-axis program that the cell places on a
SCARA (`NE0804`, plus a `dest` parameter that the SPEL+ emitter does not translate). Both
are limits of earlier backends, kept as open items; the native backend is not used by any
manifest and is refused with a message. Tests: 4.

## Results of D3.2

Done before D3.1 because it only needs the manifests; D3.1 reuses it. `links.rs` gives
one `LinkReport` per `link` (ends, medium and options, `every`, `within`, the four terms
of spec 14.8, the lowest trust level of the numbers used, and the alternatives).
Choices: transport adds the sender's output time, the medium's `transport_worst` and the
receiver's input time, each with its jitter (a deliberate worst case); the receiver
phase is the receiver's `scan_min` (D3.1 will use the program's own period when it is
longer); polling is `every`, else the sender's `scan_min`, and zero for an event-driven
medium without `every`; the value is counted as 4 bytes until D3.1 knows its type.
`NE0701` gives every term and the alternatives that meet the budget: a shorter period,
another medium both ends speak, or the event-driven form. A medium one end does not
speak, or an end that is not a device of the cell, is `NE0607`. The five corpus and
bottling links meet their budgets with generic numbers; to keep the bottling link
`plc -> arm` (25 ms) inside its budget, the robots' placeholder Modbus timing went from
5 + 2 ms to 3 + 1 ms (still generic, measured in I1 and I6). A 200 us link between two
controllers over Modbus TCP is `NE0701` and proposes `via zero_hop` (150 us). Tests: 6,
with a property test that the latency is the sum of its terms, grows one for one with
the period, and that `NE0701` fires exactly when it exceeds the budget. `NE0701` moved
from `docs/errors/reserved.md` to `docs/errors/cell.md`.

Decisions accepted by Hell on 2026-11-20: endpoints as NIR items (design decision 1),
co-simulation on the interpreter with the fidelity bounds of Phase C added to the link
budgets (decision 2), dossier as Markdown plus HTML with the PDF printed from it
(decision 3). Order: E4 (relay protocol contract) first, E1 (WASM and PyO3 bindings)
alongside D1 and D2, then D3 to D7.
