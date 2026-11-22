# Cells: several devices, one design

A **cell** is a Neuron module of kind `cell` that describes a machine built from
several devices (a PLC, a controller board, a robot arm, an FPGA board, a host
computer) and the links between them. The cell compiler splits the program over the
devices, builds each device's program with its own backend, checks the links, the
electrical wiring, the tags, the shared zones and the properties of the whole cell,
simulates all devices together, and writes a technical dossier.

This is a design aid, not a certified safety tool. It makes no claim of compliance with
IEC 61508 or ISO 13849. A hardwired E-STOP is the authority; nothing a cell generates
replaces it.

All numbers (scan times, link times, safe-state times) are `generic` placeholders until
the lab session I1 measures them. Every report repeats the trust level of the numbers it
used. See [Known limits](#known-limits).

## A small cell

```neuron
cell TwoDevices
  device ctl: nucleo_f446re @ usb
  device plc: generic_s7_1200 @ ip(192.168.0.10)
  link ctl.ready -> plc.start via modbus_tcp every 20 ms within 50 ms
  tag line_speed: f32 mm/s @ plc.%MD20, ctl.speed
```

| Item | Meaning |
|---|---|
| `device name: type @ address` | An instance of a device type from the library. |
| `place module on device` | Puts a module on a device. |
| `link a.x -> b.y via medium [every T] [within T]` | A value that crosses from one device to another. |
| `tag name: type unit @ locations` | One value with an address on each device. |
| `zone name: box(...)` and `mutex zone between a, b.out` | A shared space and who may be in it. |
| `sequence Name on device:` | A coordinated sequence of steps. |
| `verify:` | Properties of the whole cell. |

## Commands

```text
neuron cell check   <file|dir>... [--devices dir]
neuron cell build   <file|dir>... [--cell name] [-o dir] [--devices dir]
neuron cell sim     <file|dir>... [--cell name] [--for T] [--cut link@T]...
                    [--drive device.name=value@T]... [--replay out.json] [--devices dir]
neuron cell dossier <file|dir>... [--cell name] [-o dir] [--for T] [--cut ..] [--drive ..]
```

A directory is a package (its `neuron.toml` is used); files given one by one are
analyzed together. A cell places modules that may live in other files, so give all of
them. When the inputs hold more than one cell, `build`, `sim` and `dossier` need
`--cell <name>`. Exit codes: 0 success, 1 errors in the program or the cell, 2 a wrong
command line.

### `neuron cell check`

Analyzes the inputs like `neuron check`, then runs on every cell: the partition, the link
table, the tags, the electrical check, the zones and `verify:` (with the mutex proofs).
Diagnostics use the layout of the other commands:

```text
error[NE0701]: link `a.x -> b.y` via modbus_tcp: worst case 16.205 ms exceeds `within 200 us`
 --> c.neuron:4:3
  |
  |   link a.x -> b.y via modbus_tcp within 200 us
  = help: serialize 5.36 us + transport 16 ms + receiver phase 100 us + polling 100 us (generic numbers)
  = help: via zero_hop gives 150 us
```

and a summary per cell: the devices with their manifest, class, backend and modules, one
row per link (worst case, budget, trust), the mutex proof of each zone and the number of
properties proved or violated, with the shortest counterexample of each violated one.

```text
$ neuron cell check examples/corpus/cell/x01_two_devices.neuron
cell TwoDevices (examples/corpus/cell/x01_two_devices.neuron)
  devices
    ctl        nucleo_f446re          mcu    c        0 module(s), 0 sequence(s)
    plc        generic_s7_1200        plc    scl      0 module(s), 0 sequence(s)
  links
    l0  ctl.ready -> plc.start  modbus_tcp  worst 33.005 ms  budget 50 ms  ok  trust generic
  verify: 0 proved, 0 violated
ok: no cell error (all numbers are as trusted as their manifests say)
```

**`neuron check` and cells.** The analyzer no longer checks the `verify:` of a cell (a cell
property needs the links and the placement). So that nothing that used to be checked
stops being checked, `neuron check` on a package with cells also runs all the checks of
`neuron cell check`, and its errors count in the exit code. One difference: modules that
fit no device of any cell (`NE0705`) are left out of `neuron check` with a note, because
a package such as `examples/` holds many unrelated modules; `neuron cell check` reports
them. `neuron check` uses the built-in manifests and the `devices/` folder of the
package directories on the command line.

### `neuron cell build`

Writes the program of every device to `dir/<device>/` (default `build/<cell>`), exactly
what the backend of its manifest produces: SCL or ST with `PLC_tags.csv` for a PLC, C for a
controller board, Verilog with constraints for an FPGA, a SPEL+-style program for a robot,
Python for a host. Each device also gets its `fidelity.json` and `wiring.md` (when its
backend writes them), the generated link modules under `links/`, `register_map.csv`,
`links.json` (its endpoints, register map, Neuron Wire tag ids and watchdog) and
`zones.json`. The cell folder gets `links.json` (the plan of every link) and
`watchdogs.json`. The command runs the static checks first and writes nothing when they
fail, or when a backend refuses a device (the diagnostics say which device). The build is
deterministic: the same inputs give the same bytes.

### `neuron cell sim`

Runs every device's program in the reference interpreter on one cell clock and prints the
cell trace, one line per event:

```text
    100.000 ms  arm      CUT    plc.part_ready -> arm.pick
loss of l0 at 100 ms: `arm` fault at 150 ms, bound 100 ms (within)
```

Options:

| Option | Meaning |
|---|---|
| `--for T` | Simulated time (default 1 s), such as `300ms` or `2 s`. |
| `--cut link@T` | The link is lost from instant `T`. Repeatable. |
| `--drive device.name=value@T` | Writes a signal, variable or input of a device at `T` (`true`, `false`, an integer or a real). Repeatable. |
| `--replay out.json` | Writes the run as a replay for the twin (see below). |

**Naming a link in `--cut`.** `link` is either the link number as printed by
`neuron cell check` (`l0`, or just `0`; links are numbered in source order, from 0), or
the link as written in the cell (`a.ready->b.start`, spaces allowed). Examples:
`--cut l0@50ms`, `--cut "plc.motor -> arm.go@120 ms"`. An unknown link, a missing `@` or
a bad time is a command-line error (exit 2) and the message lists the links of the cell.

Every delivery takes the worst-case latency of its link (the number in the link table).
After a cut, the receiver's watchdog (see below) raises `LinkLost` and the device's own
`safe_state:` runs; the `loss of` line compares the time of that fault with the bound
(watchdog plus the device's safe-state time). A receiver with no watchdog logic (robot
controllers, see the limits) reports "no fault raised" and "NOT within".

### `neuron cell dossier`

Writes `dossier.md` and `dossier.html` (default `build/<cell>`). The HTML is one file with
inline CSS and SVG, no scripts and no external resource; print it to PDF from a browser.
The sections: summary, build status per device, wiring list, I/O list, pin and address
tables, the link table with the timing diagram, the bill of materials, the safety report
(electrical, tags, zones and mutex proofs, watchdog bounds, cell verify) and the test
report (the co-simulation, with `--for`, `--cut` and `--drive` as for `sim`). A part that
cannot be computed shows its diagnostics instead of being left out. The same inputs give
the same bytes.

## Devices and manifests

A device type is a manifest: a TOML file in `devices/<class>/` with the class (`plcs`,
`controllers`, `fpgas`, `robots`, `hosts`), the dialects it runs, its backend and profile,
its I/O families with their voltage domain, its timing per medium and its safe-state
time. Media (`modbus_tcp`, `s7`, `zero_hop`) are in `devices/media/`. The built-in set is
in the repository's `devices/` folder:

| Class | Types |
|---|---|
| PLC | `generic_s7_1200` (SCL), `generic_iec_plc` (ST, profile `yaskawa`) |
| Controller | `nucleo_f446re`, `arduino_uno_r3` (C) |
| FPGA | `generic_artix7_board` (Verilog) |
| Robot | `generic_6axis`, `generic_scara` (SPEL+-style) |
| Host | `raspberry_pi_5` (Python) |

Your own manifests: put them in a `devices/` folder in the package directory, or pass
`--devices <dir>`; a manifest with an existing name replaces the built-in one. A wrong
manifest is `NE0607`.

**Trust levels.** Each manifest says how far its numbers can be believed: `generic`
(round placeholders), `measured` (on a lab device) or `confirmed` (also checked against
the vendor documentation). A link's trust is the lowest level of the numbers it used, and
the dossier repeats it. Today everything is `generic`.

## Placement

`place m on d` puts a module on a device. A module that no cell places is placed
automatically on the device that runs its dialect and shares the most signal and `io`
names with the modules already there (ties go to the device declared first; modules are
taken in name order, so the result does not depend on file order). A cell `sequence`
without `on` runs on the first PLC, else the first host.

| Code | Meaning |
|---|---|
| `NE0704` | Impossible placement: the device cannot run the dialect, the module is written for another board, an unknown device, a module placed twice, a sequence on an FPGA. |
| `NE0705` | A module or sequence fits no device; the help says what each device lacks. |

## Links and latency

A `link` carries one value from `sender.symbol` to `receiver.symbol`. The worst-case
latency is the sum of four terms:

```text
latency = serialize + transport + receiver phase + polling
```

- **serialize**: bytes of the value on the wire (its real type) divided by the bit rate;
- **transport**: sender output time, the medium's worst case and receiver input time, each
  with its jitter (a deliberate worst case);
- **receiver phase**: wait for the receiver's next release, the longer of the manifest's
  `scan_min` and the period of the receiver's own program;
- **polling**: the sender's sampling period (`every`, else its `scan_min`; zero for an
  event-driven medium without `every`).

When the total exceeds `within`, the error is `NE0701`. It lists the four terms and the
changes that would meet the budget: a shorter `every`, another medium both devices speak,
or the event-driven form. A medium one end does not speak is `NE0607`.

## Endpoints, register maps and Neuron Wire

Each link gets generated endpoint modules, written as Neuron source in the dialect of the
device (so every backend takes them unchanged) and kept under `links/` in the device's
folder. The pattern depends on the medium: register media (Modbus TCP, S7) put the value
and its handshake registers on the receiver from holding register 100 (`register_map.csv`);
frame media use Neuron Wire frames (CRC-16, COBS on serial lines, a length prefix on TCP,
a 64-frame duplicate filter) with a tag id per link; zero-hop links use a logical pin per
link. A faster sender gets Latch+ACK, a slower one a stale flag, and an action on a robot
gets BUSY/DONE/TIMEOUT with a 1 s timeout. `links.json` lists the choice and says what the
endpoints do not do.

## Watchdog

Every receiver endpoint carries a watchdog: five heartbeats of 10 ms, 50 ms in total. When
the link is silent for that long it raises the built-in fault `LinkLost`, and the device's
own `safe_state:` applies. The bound the cell promises is the watchdog plus the device's
safe-state time; `watchdogs.json` and the dossier list it with the actuators concerned.
`neuron cell sim --cut` checks it.

## Tags

A `tag` gives one value an address on each device. The tag check reports `NE0703` for: a
type or unit that differs between devices, an address of the wrong size (`%MD20` for a
32-bit value, a bit address for a `bool`), an address family the device does not have, and
two devices that both write the value (a single writer is required).

## Electrical compatibility

A `zero_hop` link joins two I/O domains directly. If the sender's output voltage and the
receiver's input voltage differ and the medium is neither isolated nor level-shifting, the
error is `NE0801`; the help names the fix, such as `zero_hop(gpio, opto_24v)`.

## Zones and mutexes

`zone` names a box in the cell frame; `mutex zone between a, b.out` says who may enter. A
party is a device (its robot motions) or `device.output` (an output that moves something
into the zone). A constant motion target of a robot inside a zone, or an output into it,
from a device that is not a party is `NE0805`. The lock is a four-phase request/grant
protocol with an arbiter on a PLC, controller or host; the generated modules are in each
party's `links/` folder and `zones.json` lists the entries. `neuron cell check` proves
"never two parties inside" by model checking, at the delay of the lock link.

## `verify:` on the cell

The properties of a cell's `verify:` block (`never`, `eventually_within`, with machine
states such as `Seq.Step`) are checked over the composed machines of the cell and of its
placed modules, with each link as a lossless channel whose delay is its `within` budget (or
its computed worst case). A violated property is `NE0901` with the shortest
counterexample, printed as a table of ticks. Things that could not be composed are noted
(`NL0901`). Loss is not modeled here; `sim --cut` covers it.

## Co-simulation and `--cut`

See [`neuron cell sim`](#neuron-cell-sim). All devices run the reference interpreter on one
clock; values cross links after the link's latency; a cut stops all later values and starts
the receiver's watchdog.

## Replay for the twin

`--replay out.json` writes the run in the replay format (`spec/schemas/replay.schema.json`):
the devices, the links and the events in order (`signal`, `output`, `fault`, `send`,
`deliver`, `cut`, `drop`, and others), for the 3D twin to play back. The format also
carries the counterexample of a violated property (kind `counterexample`).

## Known limits

- The partition, the links and the checks are exact only to the precision of the
  manifests: **all numbers are `generic` until session I1**. The device runs (I4 to I7) are
  pending.
- **Bottling example.** `examples/` holds a cell with five classes that does not build to
  all devices: `thermostat` uses function blocks that the C backend does not support
  (`NE0610`), and `pick_robot` is a six-axis program on a SCARA arm (`NE0804`), and the
  SPEL+ emitter does not translate its `dest` parameter. `neuron cell check` and `sim` work
  on it; `build` stops with those errors. These are limits of earlier backends.
- **Robot watchdog.** The SPEL+ emitter has no background task, so a robot endpoint
  declares `alive` and `lost` but has no logic. A cut of a robot link gives "no fault
  raised" in the simulation. The robot side of the zone lock is also declared but not
  executed. Both wait for session I6.
- Cell `sequence` items are model-checked but not executed by the co-simulation, and `neuron
  cell build` writes no program for a cell that only has a sequence (see
  `x03_cycle_sequence`). FPGA circuits are not run in the co-simulation.
- Tags: `db(n).x`, `hr(n)`, `opcua`, `mqtt` and `ros2` locations, and the tag fields
  `range`, `rate` and `safe`, are not checked yet.
- Zones: joint and computed robot targets cannot be placed in a zone; the user's sequences
  are not rewritten to acquire and release the lock (the contract is listed in
  `zones.json`); lock signals are not yet wired to the register map.
- The run-time transport (the adapter that sets `alive` from Modbus, Wire or MQTT
  traffic), a start-up grace period for the watchdog and a configurable watchdog period are
  not built.
- Partition equivalence (the cell behaves like the single program) is tested for host
  cells over random periods, not proved for every dialect.
- The fidelity bounds of the Phase C translations are not added to the link latency.
