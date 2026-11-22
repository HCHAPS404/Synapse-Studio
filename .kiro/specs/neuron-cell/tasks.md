# Tasks: cell compiler (Phase D)

Order: each task ends with tests green, a `docs/kiro-log.md` entry, the roadmap row,
a Conventional Commit with `-s` and CI checked.

| Id | Task | Requirements | Needs hardware | Status |
|---|---|---|---|---|
| D1.1 | Manifest schema and loader (`devices/{plcs,controllers,fpgas,robots,hosts,media}`), trust levels, `NE0607` | D-R1, D-R11 | no (generic numbers) | done |
| D1.2 | Generic manifests for every device of the corpus cells and the bottling example; the same module on two PLC vendors | D-R1 | no | done |
| D2.1 | Partitioner: explicit `place`, automatic assignment, `NE0704`, `NE0705`, `CellPlan` | D-R2 | no | done |
| D2.2 | Per-device build through the existing backends and emitters | D-R2 | no | done |
| D3.1 | Link endpoints as NIR items, register maps, Neuron Wire frames, Latch+ACK, BUSY/DONE/TIMEOUT | D-R3 | no | done |
| D3.2 | Latency formula, `NE0701` with alternatives, link table | D-R4 | no (generic profiles; measured in I1) | done |
| D3.3 | Cell watchdog and safe state on loss | D-R5 | no | done |
| D3.4 | Tags across devices, `NE0703`, single writer | D-R6 | no | done |
| D4.1 | Zones and mutexes, `NE0805`, robot targets in zones | D-R7 | no | done |
| D4.2 | Electrical compatibility, `NE0801` | D-R7 | no | done |
| D4.3 | Cell `verify:` over the composed machines, shortest counterexample | D-R7 | no | done |
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

## Results of D4.2

`electrical.rs` checks the wired links of a link table. Only `zero_hop` media join two
I/O domains; Modbus TCP and S7 are skipped. The sender's output domain is its I/O family
with a current limit, the receiver's input domain its family with a `sink`/`source`
kind (else the first family with a voltage); domains within 0.1 V are the same. A
mismatch is `NE0801` unless the medium is isolated or the link names one of its
isolating options; the help spells the fixed `via` (`zero_hop(gpio, opto_24v)`). A
3.3 V FPGA pin on a 24 V PLC input and a 3.3 V board on a 5 V board are `NE0801`; the
corpus and bottling links, which use `opto_24v`, pass. `LinkReport` now keeps the span
of its `link`. Tests: 5. `NE0801` moved to `docs/errors/cell.md`.

## Results of D3.1
`endpoints.rs` gives one `LinkPlan` per `link` (pattern from the medium family, value
type, crossing, location of each end, fidelity notes); `build` returns them in
`CellBuild.links` and writes `links.json` (and `register_map.csv` for register media)
per device. Choices: the value type is that of the sender's signal when a placed module
declares one (`signal`, PLC `inputs`/`outputs`/`memory`, `io` as `bool`), else the
receiver's, else the default `bool` (recorded in the plan and in a note). Crossing: each
end's period is the longer of its manifest `scan_min` and the `scan` or shortest task
period of the module that declares the signal; a faster sender gets Latch+ACK, a slower
one sample + stale flag, equal periods are direct; a robot receiver over a medium that is
not a bare wire gets BUSY/DONE/TIMEOUT (timeout 1 s, `ACTION_TIMEOUT_NS`). Registers
live on the receiver, from holding register 100, one per 16 bits, in link order per
receiver: `value`, then `seq` and `ack` (Latch+ACK) or `seq`, `busy`, `done` (action),
one register each. Frame media get Wire tag id `index + 1` on both ends, pub/sub the
topic `neuron/<cell>/l<index>`, zero-hop a logical pin `l<i>_out`/`l<i>_in` plus an
`l<i>_ack` wire for Latch+ACK. Decision 1 is done as Neuron source: one generated module
per device and link side (`link_<cell>_<i>_tx|rx`, in the dialect of the device), analyzed
with the cell's sources and added to the device package, so the five backends take it
unchanged: `scl` and `st` memory variables with `%MW` addresses plus `rung`s (hold until
acknowledged, echo the sequence number, a `ton` for the action timeout), C and Python
`signal`s plus a `task`, robot `var`s, and a Verilog circuit with the real latch
(`pending = (pending or strobe) and not ack`). The sources are kept under `links/` in
each device. If a backend refuses its endpoint module the device still builds without it
and every link of that device says so in `notes` (no case in the corpus or the five-class
cell). The Neuron Wire codec is `wire.rs`: frame encode/decode (all header fields
little-endian, CRC over everything before it, 8-byte `mac` carried when the flag is set,
no HMAC), CRC-16/CCITT-FALSE, COBS with the `0x00` delimiter, the TCP length prefix, and
`DedupFilter` (64-frame window per session, wrapping sequence arithmetic; older than the
window is `Stale`). `links.rs` now costs each link with its real value size and the
period of the receiver's program (`Shape`, `latency_with`; `latency` keeps the 4-byte
default); the budgets of the corpus and bottling links stay inside `within`, no expected
number changed. Not done: connecting an endpoint to the signal of a user module and
moving values (D3.4 tags and the D6.1 co-simulation, where "the value is delivered" is
shown), the run-time Modbus/Wire/MQTT transport code, the cell watchdog (D3.3), physical
pin assignment, and a real sequence counter on zero-hop links (their `seq` is only the
echo variable). No new diagnostic code. Tests: 19 new (8 in
`d3_endpoints.rs`, 11 in `d3_wire.rs`, among them the properties WIRE-1, WIRE-2, WIRE-3,
COBS round trip, decoder never panics, register blocks never overlap), 55 in the crate
and 9 doctests.
## Results of D3.3
`watchdog.rs`: the cell watchdog is five heartbeats of 10 ms, `DEFAULT_WATCHDOG_NS` =
50 ms (spec 11.3). Placement: in the generated **receiver** endpoint module of every link
(same mechanism as D3.1: Neuron source, analyzed, added to the device package). The
module declares `l<i>_alive` (the transport adapter sets it on every value or heartbeat
that arrives) and `l<i>_lost`, and raises the built-in fault `LinkLost` when `alive` has
been false for the watchdog period; a fault applies every `safe_state:` of the device
package (spec 08 §8.2), so the device's own program reaches its safe state. Constructs
per dialect: host and mcu `when l<i>_alive == false for 50 ms: fault LinkLost` plus an
empty `safety:`; PLC a `ton` on `not alive`, rungs and `when l<i>_wd.q: fault LinkLost`
(the ST output has `neuron_fault := TRUE`); FPGA a counter circuit `l<i>_wd` (50 ms at
100 MHz, placed before the receiver circuit) with a `lost` output. Robot: the SPEL+
emitter has no rule or background task and `until` only waits on an `io` input, so the
robot endpoint declares `alive` and `lost` but no logic, and the link plan says so in
`notes`; the watchdog of a robot controller is left for session I6 (open). Plan data:
`LinkPlan.watchdog` (`period_ns`, `safe_state_ns` from the manifest `[safety]
safe_state_time`, `loss_bound_ns`, `fault`), `CellBuild.watchdogs` (one `DeviceWatchdog`
per consuming device: links, `bound_ns` = watchdog + safe-state time, and the actuators:
every PLC `outputs` variable and every output-driving `io` of the placed modules,
because a fault drives all of them and the link-to-output dataflow is not known until
links are connected to user signals), and a `watchdog` object in each device's
`links.json`. The bound is the static MOC-C2 check: the co-simulation (D6.1) cuts each
link and compares the observed time with `bound_ns`; it is not built here. Not done: the
transport adapter that sets `alive`, a boot grace period (an unconnected endpoint would
fault after 50 ms), a configurable watchdog period, a check of the watchdog against the
link budget. No new diagnostic code. Tests: 7 new in `d3_watchdog.rs` (bounds for the
corpus cells, bottling plan, two links into one device, determinism, property of the
bound), 62 in the crate and 10 doctests.
## Results of D3.4
`tags.rs`: `check_tags(nir, plan, lib)` checks every location of every `tag` of the cell
and reports `NE0703` (every problem of a mapping) or `NE0607` (a name that resolves but
is not a device of the cell; the analyzer's `NE0101` stops unknown names first).
Address sizes: `%` + area (`I`, `Q`, `M`) + size (`X` or none 1 bit, `B` 8, `W` 16, `D`
32, `L` 64) + number; a `bool` needs a bit address and any other scalar needs the
address of its own width (`f32`/`i32`/`u32` at `%MD`, `u16` at `%MW`, `u8` at `%MB`, 64
bits at `%ML`); other types have no direct address. The address must start with an
`[io]` prefix of the manifest or share its area with one (`%MD20` fits `%M` and `%MW`, so
it fits `generic_iec_plc`; it does not fit a controller). A symbol location must agree in
type and unit with the `signal`, `var`, PLC variable or `io` of that name in a module
placed on the device, when there is one (units are compared as written, so `m/s` and
`mm/s` differ). Writer rule: a location is a write location when its device's program
produces the value: a placed module assigns the symbol (any rung, task, rule, function or
state machine, found by walking the NIR statements), the address is an output `%Q...`, or
it is a memory address that a placed PLC variable declares with `@` and the module
assigns. Write locations on more than one device give one `NE0703` per tag, listing the
writers in name order, so the result does not depend on location order (property test).
Not checked yet: `db(n).x`, `hr(n)`, `opcua(..)`, `mqtt(..)`, `ros2(..)` locations, the
tag fields `range`, `rate`, `safe` of spec 11.2, and generated per-device addresses.
`check_tags` is standalone, like `check_electrical`; D7 calls it from `neuron cell
check`. `NE0703` moved from `docs/errors/reserved.md` to `docs/errors/cell.md`; the
`explain` test checks the new section. Tests: 14 in `d3_tags.rs` (corpus and bottling
tags pass, unit and type mismatch, sizes, prefixes, two writers by address and by symbol,
three properties), 76 in the crate, 12 doctests, catalogue and explain green.
## Results of D4.1
`zones.rs`: `zone_plans(nir, plan, lib)` gives one `ZonePlan` per `mutex` and the
`NE0805` diagnostics. Geometry: `box(x, y, z)` with constant ranges in mm, cm or m. A
party is `device` (its robot motions) or `device.output` (an output that moves into the
zone; checked against the outputs of the modules placed on the device, or noted as
unchecked when none is placed). Owner of the lock: a `host` that is a party, else the
first party that can run the arbiter (`plc`, `mcu`, `host`; a robot has no background
task, `hdl` is not supported). The lock transport of each party is the medium of an
existing `link` between it and the owner, else the medium both speak with the lowest
worst-case latency (`LockLink.one_way_ns`, the larger of the two directions; D4.3 turns
it into the channel delay). Protocol: four-phase request/grant, two signals per party
(`z_<zone>_req_<dev>`, `z_<zone>_grant_<dev>`); the owner's rungs
`grant_i = req_i and (grant_i or not (other grants))`, in mutex order, keep at most one
grant (`arbitrate` is the same equation; property LOCK-1). The arbiter and the party
signals are generated as Neuron source per dialect (`plc` memory and rungs, `mcu`/`host`
signals and a task, `robot` vars) and added to the device packages with the D3.1 endpoint
mechanism; `build` writes them under `links/` and `zones.json` on every party. Entries:
every `move` target of a placed robot module that is a constant Cartesian point inside
a zone box (`neuron_emit::spel::motion_targets`, base frame taken as the cell frame,
flange target, tool offset not applied) and every party output. An entry of a device that
is not a party of a mutex of that zone is `NE0805`. Not done, said plainly: the user's
sequences are not rewritten (acquire/release is a contract listed in `zones.json`), joint
and computed targets cannot be placed in a zone (counted in `unchecked_targets`), the
lock signals are not yet connected to the register map or the Wire tags of a link, the
SPEL+ emitter has no wait on a variable so the robot side of the handshake is declared
but not executed, and the model-checked proof of the mutex is D4.3. `examples/press_plc`
was added because the bottling cell's `plc.press_cylinder` was declared by no module.
Tests: 11 in `d4_zones.rs`, 87 in the crate.
## Results of D4.3
`neuron-sema` gains `verify_cell(sources, &CellModel, opts)` (the A8 checker refactored
into `verify_with`; `verify_module` and `analyze` behave as before, except that the
`verify:` properties of a `cell` module are no longer checked by `analyze`, since a cell
property needs its links). The product is the cell's own `fsm`/`sequence` items plus those
of every placed module; the inputs of a placed module are qualified with its device
(`plc.sensor`) so that a `link plc.x -> arm.y` becomes a channel between the atoms `plc.x`
and `arm.y`. Channel: a lossless FIFO with delay `0..=k` ticks (the destination reads the
source as it was `d` ticks ago; `d` grows by at most one per tick and falls only over
samples equal to the one read last, so no change is missed or reordered); `k` is the
`within` budget of the link, else its computed worst case, rounded up to the tick, at most
14 (a longer delay coarsens the tick). The tick is the fastest endpoint period of the
devices and divides every duration of the product. Loss is not modeled. Properties may
name a sequence step (`Seq.Step`) and a member machine state (`module.Machine.State`).
`neuron-cell::verify`: `verify_cell` (links to channels, `NE0901` with the counterexample
as plain data, `NL0901` notes) and `prove_zone` (the mutex of D4.1: one machine per party
with Idle, Wait, Inside, Leaving, one for the arbiter, the request and grant signals as
channels with the lock delay of the plan, and "never both inside" for each pair of
parties). x02 is proved at its real lock delay (14 ms over Modbus TCP, two ticks), also at
four ticks and with three parties. The two-phase variant (no wait for the grant to fall)
is violated at a 30 ms delay with a shortest trace, which is why the generated protocol
has four phases. The model is the protocol, not the translated rungs: the arbiter's
single-scan hand-over is one tick faster than the model's `Free` step, which does not
change the argument (a grant falls only after the request fell). Not done: the
properties are not yet read from `neuron cell check` (D7), machines of placed modules are
modeled at every tick (a slower device reacts more often than it can), and the replay
format of the counterexample is D6.2. Tests: 6 in `neuron-sema/tests/d4_cell_verify.rs`
(shortest trace, delay bound, lossless pulse, property CEX-1), 8 in
`neuron-cell/tests/d4_verify.rs` (mutex proofs, two-phase counterexample, a cell with two
sequences and a link, property MUTEX-1), 1 doctest.
Decisions accepted by Hell on 2026-11-20: endpoints as NIR items (design decision 1),
co-simulation on the interpreter with the fidelity bounds of Phase C added to the link
budgets (decision 2), dossier as Markdown plus HTML with the PDF printed from it
(decision 3). Order: E4 (relay protocol contract) first, E1 (WASM and PyO3 bindings)
alongside D1 and D2, then D3 to D7.
