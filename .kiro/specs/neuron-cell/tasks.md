# Tasks: cell compiler (Phase D)

Order: each task ends with tests green, a `docs/kiro-log.md` entry, the roadmap row,
a Conventional Commit with `-s` and CI checked.

| Id | Task | Requirements | Needs hardware | Status |
|---|---|---|---|---|
| D1.1 | Manifest schema and loader (`devices/{plcs,controllers,fpgas,robots,hosts,media}`), trust levels, `NE0607` | D-R1, D-R11 | no (generic numbers) | open |
| D1.2 | Generic manifests for every device of the corpus cells and the bottling example; the same module on two PLC vendors | D-R1 | no | open |
| D2.1 | Partitioner: explicit `place`, automatic assignment, `NE0704`, `NE0705`, `CellPlan` | D-R2 | no | open |
| D2.2 | Per-device build through the existing backends and emitters | D-R2 | no | open |
| D3.1 | Link endpoints as NIR items, register maps, Neuron Wire frames, Latch+ACK, BUSY/DONE/TIMEOUT | D-R3 | no | open |
| D3.2 | Latency formula, `NE0701` with alternatives, link table | D-R4 | no (generic profiles; measured in I1) | open |
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

Decisions accepted by Hell on 2026-11-20: endpoints as NIR items (design decision 1),
co-simulation on the interpreter with the fidelity bounds of Phase C added to the link
budgets (decision 2), dossier as Markdown plus HTML with the PDF printed from it
(decision 3). Order: E4 (relay protocol contract) first, E1 (WASM and PyO3 bindings)
alongside D1 and D2, then D3 to D7.
