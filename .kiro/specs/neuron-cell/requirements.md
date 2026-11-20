# Requirements: cell compiler (Phase D, tasks D1 to D6)

Status: draft for review (2026-11-20). Parent spec: `neuron-language`. Normative text:
`spec/language/06-time-concurrency.md` §6.11 (Standard Time Model at cell level),
`07-hardware.md` §7.8 (electrical model), `08-safety.md` (zones, `verify`),
`09-dialects.md` §9.9 (`cell`), `11-interchange.md` (tags, links, Neuron Wire),
`14-computation-models.md` §14.8 (cell MoC, properties MOC-C1 and MOC-C2). Roadmap
rows D1 to D5.

## Context

Phases A to C compile one module for one device and translate it to the languages of
PLCs, robots and hosts. A real installation is several devices that share signals,
zones and a cycle. The language already parses `cell` files (`device`, `place`, `link`,
`tag`, `zone`, `mutex`, `event`) and lowers them to NIR, and the analyzer accepts them
leniently, but nothing checks or compiles a cell: `NE0701`, `NE0703`, `NE0801` and
`NE0805` exist only in chapter 10. Phase D turns a cell into one program per device,
synthesizes both ends of every link, proves the timing and safety rules that can be
proven without hardware, and writes the technical dossier.

Everything in Phase D runs without hardware. Timing profiles are generic until session
I1 measures the lab devices; real transports (Modbus, S7, OPC UA, MQTT) run in the Edge
Daemon (E5), not here.

## User stories

- As an integrator, I want to describe a bottling cell once and get the PLC, robot, MCU
  and FPGA programs with their links, so that I do not hand-write register maps.
- As a control engineer, I want a link whose latency budget cannot be met to be rejected
  before commissioning, with a medium or period that would work.
- As a safety reviewer, I want the cell to prove that two devices never occupy a shared
  zone at once, that no 3.3 V output drives a 24 V input, and that a lost link puts the
  actuators in their safe state, with a counterexample when a rule fails.
- As a student or judge, I want the whole cell to run in simulation and to produce a
  dossier (wiring, I/O list, timing, test report) without any device connected.

## Requirements

| Id | Requirement | Acceptance |
|---|---|---|
| D-R1 | Device manifests (`devices/<class>/*.toml`) for every device class a cell names: kind, dialects it runs, the emitter or backend that builds for it, I/O with addresses and voltage domain, link media it supports, a timing profile per medium, and a trust level (`generic`, `measured`, `confirmed`) | the four corpus cells and the bottling example load their manifests; an unknown device is `NE0607`; the same module placed on two PLC manifests of different vendors builds for both |
| D-R2 | Partitioning: explicit `place m on d`, and automatic assignment of unplaced modules by dialect and capability; each device gets its program through the existing backend or emitter (C/native, Verilog, ST/SCL/PLCopen, SPEL+, Python, kernel) | the bottling cell partitions into one program per device (4 or more); a module that fits no device is a diagnostic with the devices that came closest |
| D-R3 | Link synthesis on both ends: for every `link a.x -> b.y via m [every P] [within L]` the sender and the receiver programs get the code that writes and reads the value (register map, frame layout of Neuron Wire, topic or node), with Latch+ACK on fast-to-slow crossings and BUSY/DONE/TIMEOUT for actions | the generated endpoints of every corpus link appear in both device programs and in the link table; the co-simulation shows the value delivered |
| D-R4 | Worst-case latency per link (§14.8: serialization + transport + receiver phase + polling) from the timing profiles; `NE0701` when `latency > L`, with at least one alternative (faster medium, shorter period, event-driven link) that meets `L` | a 200 µs link over Modbus TCP is `NE0701` and proposes a medium or an event-driven link that fits; every corpus link reports its computed budget |
| D-R5 | Cell watchdog (default 50 ms) and the MOC-C2 rule: every device that depends on a link enters its safe state when the link is lost | in the co-simulation, cutting a link puts every dependent actuator in its safe value within watchdog + safe-state time, for every corpus cell |
| D-R6 | Tags: `tag t: T @ d1.addr, d2.name` maps one value onto several devices; incompatible types or units are `NE0703`; two writers are an error | corpus tags check; a unit mismatch and a second writer each give their diagnostic |
| D-R7 | Safety across the cell: `zone` and `mutex` insert lock and unlock around zone entry on every device (robot motions into the zone, PLC outputs that move into it), `NE0805` when an entry is not covered; robot targets checked against the zone and the robot manifest (`NE0804`); electrical compatibility of every link endpoint (`NE0801` unless the medium has isolation or level shifting); cell-wide `verify:` properties checked by the model checker over the composed state machines and sequences | `x02_robot_zone` proves the mutex; a link 3.3 V to 24 V without `opto_24v` is `NE0801`; a violated `verify` gives a shortest counterexample trace |
| D-R8 | Cell co-simulation: every device program runs in the reference interpreter in one cell clock, links modeled with their period, latency and loss; the observable cell trace (spec 14.9) is written in a replay format the digital twin can play | every corpus cell runs; a counterexample of D-R7 replays to the same violation; the replay file validates against its JSON Schema |
| D-R9 | Technical dossier: wiring list, I/O list per device, pin and address tables, link table with latency budgets, timing diagram, bill of materials from the manifests, the safety report and the test report of the co-simulation | generated for the bottling cell; deterministic; every number in it comes from the manifests, the analysis or the co-simulation |
| D-R10 | Command line: `neuron cell check <cell>`, `neuron cell build <cell> [-o dir]`, `neuron cell sim <cell> [--for T] [--cut link@t] [--replay out.json]`, `neuron cell dossier <cell> [-o dir]` | CLI tests for each command and for each diagnostic above |
| D-R11 | No vendor material is copied; manifests use generic names and round numbers until measured; anything that needs a real device is marked for its session (I1, I4 to I7) | review of the manifests, the dossier and `docs/cells.md` |

## Correctness criteria

- **MOC-C1 (bounded delivery).** If the cell compiles, the co-simulation delivers every
  link value within `L`, or the watchdog fires.
- **MOC-C2 (safe on loss).** After a link loss, no actuator driven through that link
  stays energized beyond the watchdog interval plus the device's safe-state time.
- **Partition equivalence.** For a cell whose devices communicate only through links,
  running the partitioned programs in the co-simulation with ideal links (zero latency,
  no loss) gives the same observable trace as running all modules in one interpreter.
- **Determinism.** The same cell and manifests give byte-identical programs, link
  tables, dossier and replay files.

## Out of scope

Real transports and their drivers (Edge Daemon, E5); clock synchronization over PTP on
real hardware; measured timing profiles (session I1); drones and MAVLink (session I9
is inventory only; it needs its own spec); PDF rendering if the dossier is HTML (see
design decision 3); automatic placement that optimizes cost or wiring length (only
feasibility is required).
