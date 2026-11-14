# 01 · Overview

## 1.1 Goals

Neuron SHALL:

1. **Program devices directly**: compile to native code for microcontrollers (bare
   metal or on a vendor SDK), to PLC languages, to synthesizable HDL, to robot
   programs and to Linux user-space programs.
2. **Describe whole cells**: heterogeneous devices, the links between them, shared
   physical zones, distributed sequences, timing budgets and safety rules.
3. **Be the hub for translation**: every importer produces Neuron and every emitter
   consumes Neuron, through one canonical IR (NIR).
4. **Make costs and hazards explicit**: memory, time, communication and hardware
   access are visible in the source and checked by the compiler.
5. **Be learnable**: readable syntax, diagnostics that teach, graphical views that
   are exact projections of the text.

## 1.2 Non-goals

Neuron SHALL NOT:

- be a general-purpose application language (web services, data science);
- generate or modify safety-rated programs (fail-safe PLC programs, robot safety
  configuration); it may read their state;
- hide allocation, blocking, communication or approximation;
- have vendor-specific syntax: vendor differences live in device manifests and
  emitters, never in the grammar.

## 1.3 Architecture of the language

```
            ┌──────────── dialects (syntax + graphical views) ─────────────┐
            │ mcu │ plc │ hdl │ robot │ ui │ host │ cell │  core (portable) │
            └──┬────┬─────┬─────┬──────┬────┬──────┬───────────────────────┘
               │    │     │     │      │    │      │
            ┌──▼────▼─────▼─────▼──────▼────▼──────▼──┐
            │       models of computation (MoC)       │
            │ tasks+IRQ · scan · RTL · motion · UI ·   │
            │ host · distributed cell                  │
            └───────────────────┬─────────────────────┘
                                │
            ┌───────────────────▼─────────────────────┐
            │              Neuron Core                │
            │ types · units · memory · effects · time │
            │ resources · contracts · modules         │
            └───┬───────────────┬─────────────────┬───┘
                │               │                 │
         hardware model    safety model     interchange (NIR,
         (ch. 07)          (ch. 08)         tag dictionary, Neuron Wire)
```

- **Neuron Core** is small and has a formal semantics. Every dialect lowers to it;
  verification and translation operate on it.
- A **model of computation** defines how time advances and how concurrent activities
  interact (chapter 6 and 9).
- A **dialect** = Core + one MoC + domain vocabulary. Each file declares its dialect,
  and the compiler accepts only the constructs legal in it (`NE0501` otherwise).

| Dialect | MoC | Typical targets | Graphical views |
|---|---|---|---|
| `core` | none (pure, portable) | any (libraries) | — |
| `mcu` | event/interrupt tasks, static priorities | microcontrollers, soft cores | chip/pin view, clock tree, memory map |
| `plc` | synchronous scan cycle | PLCs, soft PLCs | Ladder, FBD, SFC/GRAFCET, variable table |
| `hdl` | synchronous RTL | FPGAs, CPLDs | RTL schematic, block design, pin planner |
| `robot` | sequential motion with asynchronous actions | industrial arms, SCARA, cartesian | 3D path, point table, timeline |
| `ui` | retained-mode UI with data binding | displays on MCUs, HMIs, web HMI | screen editor |
| `host` | threads of OS processes, soft real time | Linux SoCs, PCs | dataflow view |
| `cell` | distributed system with the Standard Time Model | the whole installation | topology, timing diagram, 3D twin |

## 1.4 Program structure

A **file** contains exactly one module or one cell. The first line is the header.

```neuron
module blink for mcu on nucleo_f446re
```

- `module <path> [for <dialect>] [on <target>]` — the dialect defaults to `core`;
  the target MAY be given in `neuron.toml` instead.
- `cell <Name>` — a cell file; its dialect is always `cell`.

A **package** is a directory with `neuron.toml`. Module paths follow directories:
`motors/stepper.neuron` is `module <pkg>.motors.stepper`.

## 1.5 A tour

**Portable library (`core`)**
```neuron
module mylib.filters

## First-order low-pass filter.
pub class LowPass:
  var state: f32 = 0.0
  let alpha: f32

  pub fn step(mut self, x: f32) -> f32:
    requires 0.0 <= self.alpha <= 1.0
    self.state = self.state + self.alpha * (x - self.state)
    return self.state
```

**Microcontroller (`mcu`)**
```neuron
module blink for mcu on nucleo_f446re

clocks:
  sysclk = 84 MHz from hse(8 MHz)

io led: gpio.output @ PA5
io btn: gpio.input(pull: up) @ PC13

var presses: u32 = 0

task blink every 500 ms (prio 1):
  led.toggle()

on interrupt btn.falling (prio 3):
  presses += 1

safety:
  safe_state:
    led.write(false)
```

**PLC (`plc`)**
```neuron
module conveyor for plc
scan 10 ms

inputs:
  start:  bool @ %I0.0
  stop:   bool @ %I0.1   # normally closed contact
  sensor: bool @ %I0.2
outputs:
  motor:  bool @ %Q0.0

let jam = ton(preset: 3 s)

rung run_motor:
  motor = (start or motor) and stop and not jam.q

rung detect_jam:
  jam.input = motor and sensor

safety:
  safe_state:
    motor = false
```

**FPGA (`hdl`)**
```neuron
module pwm8 for hdl
clock sys: 100 MHz @ pin(W5)

circuit pwm(duty: in bit<8>, pwm_out: out bit) on sys:
  reg counter: bit<8> = 0
  on rising(sys):
    counter = counter +% 1
  comb:
    pwm_out = counter < duty
```

**Robot (`robot`)**
```neuron
module pick for robot
frame table = frame(origin: xyz(300, 0, 0) mm, rot: rpy(0, 0, 0) deg)
tool gripper = tool(offset: xyz(0, 0, 85) mm, payload: 0.5 kg)

point home   = joints(0, -30, 60, 0, 60, 0) deg
point pick_a = table.xyz(120, 40, 10) mm

sequence pick_and_place(dest: point):
  move ptp to home speed 50 %
  move linear to pick_a.above(30 mm) speed 300 mm/s
  move linear to pick_a speed 50 mm/s
  run gripper.close()
  until gripper.holding timeout 1 s -> Fault
  move linear to dest speed 300 mm/s
```

**Display / HMI (`ui`)**
```neuron
module panel for ui on esp32_devkit
display lcd: ili9341(bus: spi1, size: (320, 240), rotation: 90 deg)

signal oven_temp: f32 degC

screen Main on lcd:
  label title "Bottling line" at (10, 8) font sans_16
  gauge temp bind oven_temp range 0..120 degC at (20, 40)
  button start "START" at (20, 180):
    on press:
      emit start_requested
```

**Cell (`cell`)**
```neuron
cell BottlingLine
  device ctl:  nucleo_f446re            @ usb
  device plc:  generic_s7_1200          @ ip(192.168.0.10)
  device arm:  generic_scara            @ ip(192.168.0.20)
  device fpga: generic_artix7_board     @ uart(COM4, 115200 Bd)

  place blink on ctl
  place conveyor on plc

  link fpga.bottle  -> plc.sensor   via zero_hop(gpio, opto_24v) within 15 ms
  link plc.cap_done -> arm.pick     via modbus_tcp every 10 ms within 25 ms

  zone press: box(x: 100..220 mm, y: -40..40 mm, z: 0..120 mm)
  mutex press between arm, plc.press_cylinder
```

## 1.6 Compilation model

```
.neuron ──parse──▶ AST ──resolve/check──▶ HIR ──lower──▶ NIR (.nir.json)
                                                        │
                       verify (safety, timing, model checking)
                                                        │
                    partition per device ──▶ MIR (SSA) ──▶ backends
```

The NIR is the interchange contract between tools (see `spec/schemas/`).
Every lowering step preserves source spans so that generated code carries a source
map back to `.neuron` lines and graphical elements.
