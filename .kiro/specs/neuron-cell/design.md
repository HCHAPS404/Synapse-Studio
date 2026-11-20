# Design: cell compiler (Phase D)

## Crates

| Crate | Content | Depends on |
|---|---|---|
| `neuron-cell` | manifests (D1), partitioner (D2), link synthesis and timing (D3), cell safety (D4), co-simulation and replay (D6), dossier (D5) | `neuron-nir`, `neuron-sema`, `neuron-interp`, `neuron-emit`, `neuron-codegen-c`, `neuron-codegen-hdl`, `neuron-target`, `serde`, `toml` |
| `neuron-cli` | `src/cell.rs`: `neuron cell check|build|sim|dossier` | `neuron-cell` |

`neuron-cell` reads the analyzed NIR package (the cell file plus every module it places)
and never re-parses source. Its outputs are plain data (`CellPlan`, `LinkTable`,
`Dossier`, `Replay`) so the web IDE and the twin can use them through WebAssembly later
(E1).

## D1 Device manifests

Folders under `devices/`: `plcs/`, `controllers/` (MCU boards: a manifest points at the
existing `targets/boards/*.board.toml` instead of repeating it), `fpgas/` (points at
`targets/fpga/`), `robots/` (existing, extended), `hosts/` (Raspberry Pi 5, PC), and
`media/` (one file per `via` medium).

```toml
[device]
name = "generic_s7_1200"
kind = "plc"                 # plc | mcu | fpga | robot | host
dialects = ["plc"]
backend = "st"               # st | scl | plcopen | spel | python | c | native | verilog
profile = "generic"          # emitter profile
trust = "generic"            # generic | measured | confirmed

[io]                          # address families and their voltage domain
digital_in  = { prefix = "%I", volts = 24.0, kind = "sink" }
digital_out = { prefix = "%Q", volts = 24.0, max_ma = 500 }

[timing]                      # per medium; generic until session I1
scan_min = "1 ms"
modbus_tcp = { transport = "2 ms", jitter = "1 ms" }
s7 = { transport = "1 ms", jitter = "500 us" }

[safety]
safe_state_time = "10 ms"     # time to reach the safe state after a fault
```

Media files give `bitrate`, `frame_overhead_bytes`, `transport_worst`, `isolated`
(true for `opto_24v`), `event_driven` (whether a change can be pushed without polling),
and the device kinds that can speak it. The manifest loader validates units with the
same unit table as the analyzer and returns `NE0607` for an unknown or invalid file.

Trust levels go into every report: a budget computed from `generic` numbers is labeled
as such in the dossier.

## D2 Partitioner

1. Every `place m on d` is taken as given; a dialect the device cannot run is an error
   (`NE0501`-style, new code `NE0704` "module placed on a device that cannot run it").
2. Unplaced modules are assigned to the first device whose kind runs their dialect and
   whose capabilities cover their `io` (pins, addresses); ties go to the device already
   holding the modules it shares signals with. If no device fits, `NE0705` lists the
   devices that came closest and what each lacks.
3. Each device then gets one NIR package (its modules plus the generated link
   endpoints of D3) and its backend builds it through the existing crate (C/native,
   Verilog, ST/SCL/PLCopen, SPEL+, Python, kernel). The plan (`CellPlan`) lists device,
   modules, backend, generated files and fidelity summary.

Decision 1 (to confirm): the endpoints are generated as Neuron NIR items added to the
device package, so every backend gets them for free and they are checked by the same
rules, instead of writing endpoint code per backend.

## D3 Links and timing

For each link the planner picks the endpoint pattern from the medium:

| Medium family | Sender | Receiver |
|---|---|---|
| register (`modbus_tcp`, `modbus_rtu`, `s7`) | writes the value to a holding register / DB address from a register map | reads it on its scan / task |
| frame (`uart`, `rs485`, `neuron_wire`, `can`) | Neuron Wire frame (11-interchange) with sequence number and CRC | decodes, checks CRC and sequence |
| pub/sub (`mqtt`, `ros2`, `opcua`) | publish on change or on period | subscribe |
| `zero_hop(gpio, ...)` | drives a pin | reads a pin (edge or level) |

Latency (spec 14.8): `serialize + transport + receiver_phase + polling`, with each
term taken from the manifests and the periods in the programs. `NE0701` when it exceeds
`within L`; the help text lists alternatives computed by the same formula over the
media both devices support, sorted by latency, plus "make the link event-driven" when
the medium allows it. Fast-to-slow crossings get Latch+ACK (the sender holds the value
until an ack field echoes its sequence number); actions get the BUSY/DONE/TIMEOUT
handshake (spec 06.9). Each device that consumes a link gets a watchdog task with the
cell watchdog period that enters the safe state when no fresh value arrived.

## D4 Safety across the cell

- **Zones and mutexes.** A `mutex z between a, b.x` becomes a lock owned by one
  device (the first in the mutex list, or a `host` if present) with request/grant
  links; every robot motion whose target is inside `z` (geometry from C3 and the robot
  manifest) and every output listed in the mutex is wrapped in acquire/release.
  `NE0805` for an entry the planner cannot wrap.
- **Electrical.** For each link endpoint pair, compare the output voltage domain with
  the input domain; a mismatch without an isolating or level-shifting medium is
  `NE0801`.
- **`verify:`.** The A8 model checker runs on the product of the cell's state machines
  and sequences with each link abstracted as a bounded-delay channel; a violation
  yields the shortest counterexample, which D6 turns into a replay.

## D6 Co-simulation and replay

One interpreter instance per device, stepped together on the cell clock (1 ns
resolution, each device at its own period). A link is a queue with the computed
latency, the sender's period, and optional loss (`--cut link@t`). The observable cell
trace is the union of device traces tagged with the device name. `Replay` is a JSON
file (schema in `spec/schemas/replay.schema.json`) with the cell, the devices, and the
time-ordered events; Martin's twin (F7) plays it. Decision 2 (to confirm): the
co-simulation runs NIR in the reference interpreter, not the translated code in the
virtual PLC and controllers; the translated programs are already checked against the
interpreter in Phase C, so this is sound and much faster.

## D5 Dossier

Generated from `CellPlan`, `LinkTable`, the safety report and a co-simulation run:
wiring list, I/O list per device, pin/address tables, link table with budgets and trust
level, a timing diagram (SVG), a BOM from the manifests, the safety report and the
test report. Decision 3 (to confirm): output Markdown plus a single self-contained
HTML file with the SVGs inline; a PDF is printed from the HTML (no PDF library in the
Rust build). All numbers come from the analysis; nothing is typed by hand.

## Diagnostics

`NE0701` link budget, `NE0703` tag types or units, `NE0704` placement on a device that
cannot run the dialect (new), `NE0705` no device fits (new), `NE0801` voltage levels,
`NE0804` robot workspace (existing), `NE0805` zone entry without lock. New codes go to
chapter 10 and `docs/errors/cell.md`.
