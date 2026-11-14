# 09 · Dialects

A dialect = Neuron Core + one model of computation + domain vocabulary. The header
selects it: `module <path> for <dialect> [on <target>]`. Constructs outside the
dialect are rejected with `NE0501`, which names the construct, the dialect and an
alternative.

## 9.1 Summary of legal constructs

| Construct | core | mcu | plc | hdl | robot | ui | host | cell |
|---|---|---|---|---|---|---|---|---|
| functions, types, consts | ✔ | ✔ | ✔ | ✔ (synthesizable) | ✔ | ✔ | ✔ | ✔ |
| `var` static state | — | ✔ | ✔ | `reg` instead | ✔ | ✔ | ✔ | — |
| `signal` | — | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ (tags) |
| `task every / on` | — | ✔ | ✔ (inside the scan) | — | ✔ | — | ✔ | — |
| `on interrupt` | — | ✔ | ✔ (hardware interrupt OBs, if available) | — | — | — | — | — |
| `fsm`, `when` | — | ✔ | ✔ | ✔ (as RTL FSM) | ✔ | ✔ | ✔ | ✔ |
| `sequence` | — | ✔ | ✔ | — | ✔ | — | ✔ | ✔ |
| `io`, `clocks`, `uses regs` | — | ✔ | `inputs/outputs` | `clock`, ports | robot I/O | `display` | OS devices | `device` |
| floats | ✔ | ✔ | ✔ | — | ✔ | ✔ | ✔ | ✔ |
| recursion, heap | — | — | — | — | — | — | ✔ | — |
| `asm`, `extern "C"` | — | ✔ | — | — | — | ✔ | ✔ | — |

## 9.2 `core` — portable code

No hardware, no tasks, no static state. Pure functions, types and classes. This is
the dialect of most of the standard library and of user libraries shared between
targets. A `core` module can be used from every other dialect (in `hdl` only its
synthesizable subset).

## 9.3 `mcu` — microcontrollers

**MoC:** fixed-priority preemptive tasks and interrupts on each core (06), SRP
resources, base tick 1 ms by default.

Additions: `io`, `clocks`, `uses regs`, `on interrupt`, `@section`, `@place`,
`trusted asm`, `trusted extern`.

Graphical views: **chip view** (package with pin functions and conflicts),
**clock tree** configurator, **memory map** (flash, RAM, stack per task),
**task timeline** (schedulability).

Platform runtimes: bare metal (default), vendor SDK where the radio or USB stack
requires it, RTOS ports.

## 9.4 `plc` — programmable logic controllers

**MoC:** synchronous scan cycle (IEC 61131-3): read all inputs, execute the program,
write all outputs. `scan P` sets the cycle.

```neuron
module press for plc
scan 10 ms

inputs:
  start:   bool @ %I0.0
  guard:   bool @ %I0.1
  pos:     i16 mm @ %IW64
outputs:
  valve:   bool @ %Q0.0
  lamp:    bool @ %Q0.1
memory:
  retain cycles: u32 @ %MD10

let t_hold = ton(preset: 2 s)
let edge   = r_trig()

rung start_press:
  edge.input = start
  set valve when edge.q and guard
  reset valve when t_hold.q or not guard

rung hold_timer:
  t_hold.input = valve and pos >= 120 mm

rung count:
  cycles += 1 when t_hold.q.rising
```

- `inputs:`, `outputs:` and `memory:` sections declare variables with optional IEC
  addresses (`@ %I0.0`). Without addresses, the emitter assigns symbolic tags.
- `rung name:` contains **only** boolean networks: assignments of boolean
  expressions (coils), `set x when c`, `reset x when c` and calls of function blocks.
  Each rung maps one-to-one to a Ladder rung and an FBD network.
- `retain` variables keep their value across power cycles.
- Standard function blocks come from `std.logic`: `ton`, `tof`, `tp`, `ctu`, `ctd`,
  `ctud`, `r_trig`, `f_trig`, `sr`, `rs`, plus `std.control.pid`.
- `sequence` maps to SFC; `fsm` maps to SFC or CASE-based ST.

Graphical views: **Ladder**, **FBD**, **SFC/GRAFCET**, **variable table**, **cross
reference**.

## 9.5 `hdl` — FPGAs and digital logic

**MoC:** synchronous register-transfer level. Values change only at clock edges;
`comb:` blocks are combinational and must be complete (no inferred latches).

```neuron
module uart_tx for hdl
clock clk: 100 MHz @ pin(W5)

circuit tx(data: in bit<8>, start: in bit, line: out bit, busy: out bit) on clk:
  const divisor = clk.freq / 115200 Bd

  reg shift: bit<10> = 10'b11_1111_1111
  reg count: bit<4>  = 0
  reg baud:  bit<16> = 0

  on rising(clk):
    if start and count == 0:
      shift = 1'b1 ++ data ++ 1'b0          # stop, data, start bits
      count = 10
      baud = 0
    elif count != 0:
      if baud == divisor - 1:
        shift = 1'b1 ++ shift[9:1]
        count = count - 1
        baud = 0
      else:
        baud = baud + 1

  comb:
    line = shift[0]
    busy = count != 0

sync start_async -> start via two_ff
```

- `clock name: frequency [@ pin(...)]` declares a clock domain. Every `reg` and every
  `circuit` belongs to one domain.
- `circuit` ports have directions `in`, `out`, `inout` and `bit<N>`/`sbit<N>` or
  struct types of bits. `instance u = tx(data: d, start: s)` instantiates.
- `reg` declares a register with reset value; `wire` a combinational net.
- Bit operations: slicing `x[7:0]`, concatenation `++`, replication `x ** 4`
  (on bit vectors), reduction `x.any()`, `x.all()`, `x.parity()`.
- **Clock domain crossing** without `sync` is `NE0702`. `sync a -> b via two_ff |
  latch_ack | fifo<depth>` generates the synchronizer.
- Loops unroll at compile time; floats, recursion and dynamic sizes are illegal.
- Pins carry I/O standards: `@ pin(W5, io: LVCMOS33)`.

Graphical views: **RTL schematic**, **block design**, **pin planner by bank**,
**timing report** (from synthesis feedback).

## 9.6 `robot` — manipulators

**MoC:** sequential motion programs with asynchronous actions; motion statements take
time and are implicit sequence steps.

```neuron
module palletize for robot
frame pallet = frame(origin: xyz(400, -200, 0) mm, rot: rpy(0, 0, 90) deg)
tool vacuum  = tool(offset: xyz(0, 0, 120) mm, payload: 1.2 kg)
io suction: gpio.output @ out(3)
io part_ok: gpio.input  @ in(5)

point home = joints(0, -20, 70, 0, 40, 0) deg

sequence stack(rows: u8, cols: u8):
  move ptp to home speed 60 %
  for r in 0..rows:
    for c in 0..cols:
      let p = pallet.xyz(c * 80, r * 60, 0) mm
      move linear to p.above(50 mm) speed 400 mm/s blend 10 mm
      move linear to p speed 80 mm/s
      suction.write(true)
      until part_ok timeout 500 ms -> Retry
      move linear to p.above(50 mm) speed 400 mm/s
```

- `frame`, `tool` and `point` (in joints or Cartesian with orientation) are typed
  geometric values with units.
- `move ptp | linear | arc to P [via Q] [speed v] [accel a] [blend r] [tool t]
  [frame f]`.
- **Every target point is checked against the workspace bounds and joint limits of
  the robot manifest before code is emitted** (`NE0804`); points computed at runtime
  get runtime checks that raise `fault Workspace`.
- Zones from the cell (`mutex`) insert lock/unlock around entry automatically.

Graphical views: **3D path**, **point table**, **timeline** with cycle time.

## 9.7 `ui` — displays and HMIs

**MoC:** retained-mode UI: widgets bound to signals; the renderer redraws only what
changed, at the display rate.

- `display name: driver(bus: ..., size: (w, h), rotation: ...)` binds a panel driver
  from `std.display` (ILI9341, ST7789, ST7735, ILI9488, GC9A01, SSD1306, HD44780,
  e-paper, and remote HMIs).
- `screen Name on display:` contains widgets: `label`, `button`, `toggle`, `slider`,
  `gauge`, `bar`, `chart`, `led`, `list`, `image`, `keypad`, `numeric_input`, and
  layout containers `row`, `column`, `grid`.
- Widget options: `at (x, y)`, `size (w, h)`, `bind expr`, `range`, `font`, `color`,
  `theme`; events with `on press:`, `on change:`.
- The **render strategy** (full framebuffer, tiles or partial updates) is chosen from
  the target RAM (MEM-1); if no strategy fits, `NE0604` explains the shortfall.
- The same `ui` module can be emitted as a web HMI for supervision.

Graphical views: **screen editor** (WYSIWYG).

## 9.8 `host` — Linux SoCs and PCs

**MoC:** OS threads and async I/O with soft real-time only. Heap collections,
recursion and files are allowed. Hard deadlines are rejected with `NE0405` and the
suggestion to place that logic on an `mcu` or `hdl` device. Typical use: vision,
gateways, logging, ROS 2 nodes.

## 9.9 `cell` — the whole installation

**MoC:** distributed system under the Standard Time Model (06).

| Item | Form |
|---|---|
| Device | `device plc: generic_s7_1200 @ ip(192.168.0.10)` |
| Placement | `place conveyor on plc` |
| Link | `link a.x -> b.y via medium [every P] [within L]` |
| Tag | `tag line_speed: f32 mm/s @ plc.%MD20, ctl.speed, hmi.speed` |
| Zone | `zone press: box(x: 100..220, y: -40..40, z: 0..120) mm` |
| Mutex | `mutex press between arm, plc.press_cylinder` |
| Event | `event bottle_seen: timestamp, encoder: u32 from fpga` |
| Sequence | `sequence Cycle on plc:` coordinating devices |
| Safety | cell-wide `safety:` and `verify:` |

Media for `via`: `zero_hop(gpio | uart | can, isolation?)`, `uart`, `rs485`,
`modbus_rtu`, `modbus_tcp`, `s7`, `opcua`, `mqtt`, `can`, `canopen`, `tcp`,
`ros2`, `neuron_wire`.

Graphical views: **topology**, **timing diagram** (latency budget per link),
**3D digital twin**.

## 9.10 Views are projections

Every graphical view is a **bidirectional projection** of the source: editing the view
edits the `.neuron` text and vice versa; positions live in `.layout.json`.
A construct that a view cannot represent appears in it as an opaque *code block*;
information is never lost.

> **Property VIEW-1.** For every view `V` and source `s`, `text(V(s))` parses to a
> program structurally equal to `s`.
